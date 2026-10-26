import { Fragment, useEffect, useRef, useState } from "react";
import type { WebSearchProviderStatus } from "../../generated/contract";
import { openLink } from "../../native";
import { ChevronRightIcon, PlusIcon } from "../../icons";
import { envToLines, linesToEnv } from "../../envLines";
import { mcpServerDetail, type McpHttpAuth, type McpServer } from "../../mcp";
import type { McpOAuthStatus } from "../../wire";
import { BackKey, Band } from "../../ui/Band";
import { Chips } from "../../ui/Chips";
import { Fold } from "../../ui/Fold";
import { Refusal } from "../../ui/Refusal";
import { Scroll } from "../../ui/Scroll";
import { wire } from "../../wire";
import { McpPasteBack } from "../McpPasteBack";
import { onServer } from "../../serverFiles";

type AuthMode = "none" | "oauth" | "bearer" | "header";

type ServerDraft = {
	name: string;
	kind: "stdio" | "http";
	command: string;
	url: string;
	authMode: AuthMode;
	/** The header a custom-header server sends its token in. */
	headerName: string;
	/** A token to save; empty on an edit keeps the one already saved. Never part of the server. */
	secret: string;
};

const EMPTY_DRAFT: ServerDraft = { name: "", kind: "stdio", command: "", url: "", authMode: "none", headerName: "", secret: "" };

function authModeOf(auth: McpHttpAuth): AuthMode {
	return auth.mode === "oauth" || auth.mode === "bearer" || auth.mode === "header" ? auth.mode : "none";
}

function headerNameOf(auth: McpHttpAuth): string {
	const name = (auth as { name?: unknown }).name;
	return typeof name === "string" ? name : "";
}

/** A server that sends a pasted token, kept in the vault rather than in the server. */
function holdsToken(auth: McpHttpAuth): boolean {
	return auth.mode === "bearer" || auth.mode === "header";
}

/**
 * Tools: the MCP servers the room has, drawn the way Providers is. An add
 * row over the list opens the form in place; a row opens the server's own
 * page, where it is edited and, at the foot, removed. Which teammate may
 * use a server is a different question, answered on that teammate.
 */
export function ToolsSection({
	servers,
	onBack,
}: {
	servers: McpServer[];
	onBack?: (() => void) | undefined;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [adding, setAdding] = useState(false);
	const [open, setOpen] = useState<string | null>(null);
	const [writing, setWriting] = useState(false);

	const persist = async (next: McpServer[]): Promise<boolean> => {
		setWriting(true);
		setRefusal(null);
		try {
			await wire.command("settings.update", { patch: { mcpServers: next } });
			return true;
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			return false;
		} finally {
			setWriting(false);
		}
	};

	const opened = servers.find((one) => one.id === open);
	if (opened !== undefined) {
		return (
			<ServerPage
				server={opened}
				writing={writing}
				onBack={() => setOpen(null)}
				onSave={(next) => void persist(servers.map((one) => (one.id === opened.id ? next : one))).then((ok) => ok && setOpen(null))}
				onRemove={() => void persist(servers.filter((one) => one.id !== opened.id)).then((ok) => ok && setOpen(null))}
				refusal={refusal}
			/>
		);
	}

	return (
		<div className="pane">
			<Band column>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Tools</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{adding && (
						<ServerForm
							title="Add a server"
							submit="Add server"
							writing={writing}
							onSubmit={(next) => void persist([...servers, next]).then((ok) => ok && setAdding(false))}
							onCancel={() => setAdding(false)}
						/>
					)}
					<section>
						<h3 className="group-title">MCP servers</h3>
						<div className="grouped">
							{!adding && (
								<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
									<PlusIcon />
									Add server
								</button>
							)}
							{servers.length === 0 ? (
								<p className="group-row text-sm text-ink-3">No servers yet.</p>
							) : (
								servers.map((server) => (
									<button
										key={server.id}
										type="button"
										className="group-row group-row-choice w-full text-left"
										onClick={() => setOpen(server.id)}
									>
										<span className="group-row-text">
											<span className="group-row-title">{server.name}</span>
											<span className="group-row-detail">
												{server.type === "stdio" ? "Command" : "HTTP"} · <span className="font-mono">{mcpServerDetail(server)}</span>
											</span>
										</span>
										<ChevronRightIcon className="shrink-0 text-ink-3" />
									</button>
								))
							)}
						</div>
						<p className="group-hint">Grant servers per teammate, in its pane.</p>
					</section>
					<WebSearchGroup />
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/**
 * Web search: every teammate has it, keyless. A row per provider, tried in
 * this order: the switch is the desk's (off stays off for every teammate).
 * A key is optional, so it is one quiet action on the row; the field opens
 * only to add or change one, and the key goes to the keychain, never shown
 * again. A provider with a key is tried first.
 */
function WebSearchGroup() {
	const [providers, setProviders] = useState<WebSearchProviderStatus[] | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [editing, setEditing] = useState<string | null>(null);

	useEffect(() => {
		void wire
			.command("websearch.status", {})
			.then((status) => setProviders(status.providers))
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	const apply = async (run: () => Promise<{ providers: WebSearchProviderStatus[] }>): Promise<boolean> => {
		setBusy(true);
		setRefusal(null);
		try {
			setProviders((await run()).providers);
			return true;
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			return false;
		} finally {
			setBusy(false);
		}
	};

	return (
		<section>
			<h3 className="group-title">Web search</h3>
			<div className="grouped">
				{providers === null ? (
					<p className="group-row text-sm text-ink-3">{refusal === null ? "…" : "Not available."}</p>
				) : (
					providers.map((one) => (
						<Fragment key={one.provider}>
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{one.name}</span>
									<span className="group-row-detail">{one.hasKey ? "Your key" : "Keyless"}</span>
								</span>
								<button
									type="button"
									className="control btn-quiet btn-sm text-accent-ink"
									aria-expanded={editing === one.provider}
									disabled={busy}
									onClick={() => setEditing((was) => (was === one.provider ? null : one.provider))}
								>
									{editing === one.provider ? "Cancel" : one.hasKey ? "Change key" : "Add key"}
								</button>
								<input
									type="checkbox"
									role="switch"
									className="switch"
									aria-label={`Search with ${one.name}`}
									checked={one.enabled}
									disabled={busy}
									onChange={(event) => void apply(() => wire.command("websearch.set_enabled", { provider: one.provider, enabled: event.target.checked }))}
								/>
							</div>
							{editing === one.provider && (
								<WebSearchKey
									provider={one}
									busy={busy}
									onSave={(key) =>
										apply(() => wire.command("websearch.set_key", key === null ? { provider: one.provider } : { provider: one.provider, key })).then((saved) => {
											if (saved) setEditing(null);
											return saved;
										})
									}
								/>
							)}
						</Fragment>
					))
				)}
			</div>
			<p className="group-hint">Tried in order, one with a key first. Off here is off for every teammate.</p>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** One provider's key, opened from its row: saved with Save or Enter, then never shown again. */
function WebSearchKey({
	provider,
	busy,
	onSave,
}: {
	provider: WebSearchProviderStatus;
	busy: boolean;
	onSave(key: string | null): Promise<boolean>;
}) {
	const [draft, setDraft] = useState("");
	const key = draft.trim();

	return (
		<form
			className="group-row gap-2"
			onSubmit={(event) => {
				event.preventDefault();
				if (key !== "") void onSave(key);
			}}
		>
			<input
				type="password"
				className="field min-w-0 flex-1 font-mono text-sm"
				aria-label={`${provider.name} API key`}
				placeholder={provider.hasKey ? "Paste a new key" : `${provider.name} API key`}
				autoComplete="new-password"
				// biome-ignore lint/a11y/noAutofocus: the field opens on a press of Add key or Change key, which wants a key typed next.
				autoFocus
				spellCheck={false}
				value={draft}
				disabled={busy}
				onChange={(event) => setDraft(event.target.value)}
			/>
			<button type="submit" className="control btn btn-primary shrink-0" disabled={busy || key === ""}>
				Save
			</button>
			{provider.hasKey && (
				<button type="button" className="control btn-quiet shrink-0 text-danger" disabled={busy} onClick={() => void onSave(null)}>
					Remove
				</button>
			)}
		</form>
	);
}

/** One server's page: its fields, and the way out at the foot. */
function ServerPage({
	server,
	writing,
	refusal,
	onBack,
	onSave,
	onRemove,
}: {
	server: McpServer;
	writing: boolean;
	refusal: string | null;
	onBack(): void;
	onSave(next: McpServer): void;
	onRemove(): void;
}) {
	return (
		<div className="pane">
			<Band column>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{server.name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<ServerForm title="Server" submit="Save" server={server} writing={writing} onSubmit={onSave} />
					{server.type === "http" && server.auth.mode === "oauth" && <McpOAuthControls server={server} />}
					{server.type === "http" && holdsToken(server.auth) && <McpTokenControls server={server} />}
					<section>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">Remove server</span>
									<span className="group-row-detail">Teammates that used it lose its tools on their next start.</span>
								</span>
								<button type="button" className="control btn-quiet text-danger" disabled={writing} onClick={onRemove}>
									Remove
								</button>
							</div>
						</div>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

export function McpOAuthControls({ server }: { server: Extract<McpServer, { type: "http" }> }) {
	const [status, setStatus] = useState<McpOAuthStatus | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const refresh = async () => {
		try {
			setStatus(await wire.command("mcp.auth_status", { serverId: server.id }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	// biome-ignore lint/correctness/useExhaustiveDependencies: refresh reads only server.id, which is what restarts the poll.
	useEffect(() => {
		void refresh();
		const timer = window.setInterval(() => void refresh(), 2_000);
		return () => window.clearInterval(timer);
	}, [server.id]);

	const signIn = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			const next = await wire.command("mcp.auth_start", { serverId: server.id });
			setStatus(next);
			if (next.authorizationUrl) await openLink(next.authorizationUrl);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const signOut = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("mcp.auth_sign_out", { serverId: server.id });
			await refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const state = status?.status ?? "signed_out";
	const detail =
		state === "signed_in"
			? "Signed in."
			: state === "pending"
				? "Waiting for consent in your browser…"
				: state === "failed"
					? "Sign-in failed."
					: "No saved sign-in.";

	return (
		<section>
			<h3 className="group-title">OAuth sign-in</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">
							{state === "signed_in" ? "Signed in" : state === "pending" ? "Signing in…" : state === "failed" ? "Sign-in needs attention" : "Signed out"}
						</span>
						<span className="group-row-detail">{detail}</span>
					</span>
					<div className="flex shrink-0 gap-2">
						<button type="button" className="control btn-primary" disabled={busy || state === "pending"} onClick={() => void signIn()}>
							{state === "signed_in" ? "Reconnect" : "Sign in"}
						</button>
						<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void signOut()}>
							Sign out
						</button>
					</div>
				</div>
			</div>
			{/* On a server's desk the provider's redirect lands on the wrong computer's loopback (BRO-154). */}
			{onServer() && state === "pending" && status?.loginId !== undefined && (
				<McpPasteBack loginId={status.loginId} onStatus={setStatus} />
			)}
			{state === "failed" && status?.error !== undefined && <Refusal message="Sign-in failed." detail={status.error} />}
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** Whether the vault holds a token for this server, and the way to forget it. */
function McpTokenControls({ server }: { server: Extract<McpServer, { type: "http" }> }) {
	const [saved, setSaved] = useState<boolean | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const refresh = async () => {
		try {
			const status = await wire.command("mcp.auth_status", { serverId: server.id });
			setSaved(status.status === "signed_in");
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	// biome-ignore lint/correctness/useExhaustiveDependencies: refresh reads only server.id, which is what restarts the poll.
	useEffect(() => {
		void refresh();
		const timer = window.setInterval(() => void refresh(), 2_000);
		return () => window.clearInterval(timer);
	}, [server.id]);

	const forget = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("mcp.auth_sign_out", { serverId: server.id });
			await refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<section>
			<h3 className="group-title">Token</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">{saved === true ? "Token saved" : saved === false ? "No token saved" : "…"}</span>
						<span className="group-row-detail">
							{saved === true ? "Sent on every request." : "Not connected until a token is saved."}
						</span>
					</span>
					<button type="button" className="control btn-quiet text-danger" disabled={busy || saved !== true} onClick={() => void forget()}>
						Forget token
					</button>
				</div>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** The server's fields: one form for adding and for the page. */
function ServerForm({
	title,
	submit,
	server,
	writing,
	onSubmit,
	onCancel,
}: {
	title: string;
	submit: string;
	server?: McpServer;
	writing: boolean;
	onSubmit(next: McpServer): void;
	onCancel?: () => void;
}) {
	const [draft, setDraft] = useState<ServerDraft>(() =>
		server === undefined
			? EMPTY_DRAFT
			: server.type === "stdio"
				? { name: server.name, kind: "stdio", command: [server.command, ...server.args].join(" "), url: "", authMode: "none", headerName: "", secret: "" }
				: {
					name: server.name,
					kind: "http",
					command: "",
					url: server.url,
					authMode: authModeOf(server.auth),
					headerName: headerNameOf(server.auth),
					secret: "",
				},
	);
	const [refusal, setRefusal] = useState<string | null>(null);
	const savedLaunch = server?.type === "stdio" && (!!server.credentialRef || !!server.launchValuesPending);
	const [replaceLaunch, setReplaceLaunch] = useState(false);
	const [environment, setEnvironment] = useState(() => (server?.type === "stdio" ? envToLines(server.env) : ""));
	const [showEnvironment, setShowEnvironment] = useState(false);
	const keepLaunch = savedLaunch && !replaceLaunch;
	const nameField = useRef<HTMLInputElement>(null);
	useEffect(() => {
		if (server === undefined) nameField.current?.focus();
	}, [server]);

	const takesToken = draft.kind === "http" && (draft.authMode === "bearer" || draft.authMode === "header");
	/* A new token server needs its token now; an edit may keep the saved one. */
	const ready =
		draft.name.trim().length > 0 &&
		(draft.kind === "stdio" ? draft.command.trim().length > 0 : draft.url.trim().length > 0) &&
		(!takesToken || server !== undefined || draft.secret.length > 0) &&
		(draft.kind !== "http" || draft.authMode !== "header" || draft.headerName.trim().length > 0);

	const submitDraft = async () => {
		const name = draft.name.trim();
		let next = draft.kind === "stdio" ? stdioFromDraft(name, draft.command, server) : httpFromDraft(name, draft.url, draft, server);
		if (next.type === "stdio" && !keepLaunch) {
			const parsed = linesToEnv(environment);
			if ("error" in parsed) {
				setRefusal(parsed.error);
				setShowEnvironment(true);
				return;
			}
			const { credentialRef: _savedReference, launchValuesPending: _pendingValues, ...launch } = next;
			next = { ...launch, env: parsed.env };
		}
		/* The token goes to the vault first, so the settings write that
		 * follows reattaches teammates with it in hand. */
		if (next.type === "http" && holdsToken(next.auth) && draft.secret.length > 0) {
			setRefusal(null);
			try {
				await wire.command("mcp.secret_set", { serverId: next.id, url: next.url, secret: draft.secret });
			} catch (error) {
				setRefusal(error instanceof Error ? error.message : String(error));
				return;
			}
			setDraft({ ...draft, secret: "" });
		}
		onSubmit(next);
	};

	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (!ready || writing) return;
				void submitDraft();
			}}
		>
			<h3 className="group-title">{title}</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="w-24 shrink-0 text-sm text-ink-2">Runs</span>
					<Chips
						value={draft.kind}
						choices={[
							{ id: "stdio", name: "On this computer", title: "A command Hotline starts and talks to over stdio" },
							{ id: "http", name: "At a URL", title: "A server reached over HTTP" },
						]}
						label="Where the server runs"
						onChange={(kind) => setDraft({ ...draft, kind })}
					/>
				</div>
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-name">
						Name
					</label>
					<input
						id="tool-name"
						ref={nameField}
						className="field flex-1"
						value={draft.name}
						onChange={(event) => setDraft({ ...draft, name: event.target.value })}
					/>
				</div>
				{draft.kind === "stdio" ? (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-command">
							Command
						</label>
						<input
							id="tool-command"
							readOnly={keepLaunch}
							className="field flex-1 font-mono text-sm"
							spellCheck={false}
							placeholder="npx -y @modelcontextprotocol/server-filesystem /some/path"
							value={draft.command}
							onChange={(event) => setDraft({ ...draft, command: event.target.value })}
						/>
					</div>
				) : (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-url">
							URL
						</label>
						<input
							id="tool-url"
							className="field flex-1 font-mono text-sm"
							spellCheck={false}
							placeholder="https://example.com/mcp"
							value={draft.url}
							onChange={(event) => setDraft({ ...draft, url: event.target.value })}
						/>
					</div>
				)}
				{draft.kind === "stdio" && savedLaunch && (
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">{server?.type === "stdio" && server.launchValuesPending ? "Launch values need credential migration" : "Launch values stored securely"}</span>
							<span className="group-row-detail">{keepLaunch ? server?.type === "stdio" && server.launchValuesPending ? "Unlock the OS credential store and retry this source, or replace its launch values." : "Saved arguments and environment variables are kept when you rename this source." : "Enter the complete command and environment to replace the saved launch values."}</span>
						</span>
						<button type="button" className="control btn-quiet" onClick={() => {
							setReplaceLaunch(!replaceLaunch);
							if (replaceLaunch && server?.type === "stdio") setDraft({ ...draft, command: server.command });
						}}>{keepLaunch ? "Replace launch values" : "Keep saved values"}</button>
					</div>
				)}
				{draft.kind === "stdio" && !keepLaunch && (
					<Fold
						label
						title="Environment"
						value={(() => {
							const parsed = linesToEnv(environment);
							const names = "env" in parsed ? Object.keys(parsed.env) : [];
							return names.length === 0 ? "None" : names.join(", ");
						})()}
						action={environment.trim() === "" ? "Add" : "Change"}
						open={showEnvironment}
						onToggle={() => setShowEnvironment((was) => !was)}
					>
						<textarea
							id="tool-environment"
							aria-label="Environment variables"
							className="field font-mono text-sm"
							rows={3}
							spellCheck={false}
							placeholder={"API_KEY=…\nROOT=/some/path"}
							value={environment}
							onChange={(event) => setEnvironment(event.target.value)}
						/>
						<p className="hint">One NAME=value per line. Kept in the keychain.</p>
					</Fold>
				)}
				{draft.kind === "http" && server?.type === "http" && server.urlNeedsRepair && (
					<div className="group-row text-sm text-ink-2">Re-enter the endpoint without credentials, a query, or a fragment. Put tokens in the authentication fields below.</div>
				)}
				{draft.kind === "http" && (
					<div className="group-row">
						<span className="w-24 shrink-0 text-sm text-ink-2">Signs in</span>
						<Chips
							value={draft.authMode}
							choices={[
								{ id: "none", name: "No sign-in", title: "Connect without credentials" },
								{ id: "oauth", name: "With an account", title: "Sign in on the server's own page (OAuth 2.1)" },
								{ id: "bearer", name: "Token", title: "Send a token you paste as Authorization: Bearer" },
								{ id: "header", name: "Header", title: "Send a token you paste in a header you name" },
							]}
							label="How the server signs in"
							onChange={(authMode) => setDraft({ ...draft, authMode })}
						/>
					</div>
				)}
				{draft.kind === "http" && draft.authMode === "header" && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-header">
							Header
						</label>
						<input
							id="tool-header"
							className="field flex-1 font-mono text-sm"
							spellCheck={false}
							placeholder="X-API-Key"
							value={draft.headerName}
							onChange={(event) => setDraft({ ...draft, headerName: event.target.value })}
						/>
					</div>
				)}
				{takesToken && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-token">
							Token
						</label>
						<input
							id="tool-token"
							type="password"
							className="field flex-1 font-mono text-sm"
							autoComplete="off"
							placeholder={server === undefined ? "Paste the token" : "Saved · paste to replace"}
							value={draft.secret}
							onChange={(event) => setDraft({ ...draft, secret: event.target.value })}
						/>
					</div>
				)}
				<div className="group-row justify-end">
					{onCancel !== undefined && (
						<button type="button" className="control btn-quiet" disabled={writing} onClick={onCancel}>
							Cancel
						</button>
					)}
					<button type="submit" className="control btn-primary" disabled={writing || !ready}>
						{writing ? "Saving…" : submit}
					</button>
				</div>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
			<p className="group-hint">Kept on this machine.</p>
		</form>
	);
}

/** Renaming a source keeps its saved launch reference until explicitly replaced. */
function stdioFromDraft(name: string, commandLine: string, previous?: McpServer): McpServer {
	const [command, ...args] = commandLine.trim().split(/\s+/);
	const id = previous?.id ?? crypto.randomUUID();
	if (previous?.type === "stdio" && (previous.credentialRef || previous.launchValuesPending) && commandLine.trim() === previous.command) {
		return { ...previous, name };
	}
	const env = previous?.type === "stdio" ? previous.env : undefined;
	return env
		? { id, type: "stdio", name, command: command ?? "", args, env }
		: { id, type: "stdio", name, command: command ?? "", args };
}

/**
 * The server as settings hold it: the auth names a mode, never a token. An
 * edit that stays on OAuth keeps the scopes and client the server already
 * had; a pasted token lives in the vault and is not here to keep.
 */
function httpFromDraft(name: string, url: string, draft: ServerDraft, previous?: McpServer): McpServer {
	const auth: McpHttpAuth =
		draft.authMode === "oauth"
			? previous?.type === "http" && previous.auth.mode === "oauth"
				? previous.auth
				: { mode: "oauth" }
			: draft.authMode === "bearer"
				? { mode: "bearer" }
				: draft.authMode === "header"
					? { mode: "header", name: draft.headerName.trim() }
					: { mode: "none" };
	return { id: previous?.id ?? crypto.randomUUID(), type: "http", name, url: url.trim(), auth };
}
