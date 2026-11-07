import { useEffect, useReducer, useRef, useState } from "react";
import type { Credential, CredentialKind, Provider, ProviderJob } from "../generated/contract";
import { CheckIcon, ChevronRightIcon, CopyIcon } from "../icons";
import { openLink, writeClipboard } from "../native";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { formOf, restingAfter, signInBusy, signInStep } from "../signIn";
import { wire } from "../wire";
import { CustomProviderForm } from "./CustomProviderForm";

const SIGN_IN_FAILED = "Sign-in failed";

/**
 * Connecting one provider, in place: the method when it offers more than
 * one, then the key field, the server URL, the sign-in wait, or the custom
 * connection form. Settings › Providers and the welcome pane both render
 * this, so a key is pasted the same way on the first day as on any other.
 *
 * The credential is handed up once it exists and its models are read; what
 * to do with it — list it, open its page, move on to the first teammate —
 * is the caller's. Cancelling a sign-in cancels it on the core too.
 */
export function ConnectProvider({
	provider,
	replacing,
	label,
	onConnected,
	onCancel,
}: {
	provider: Provider;
	/** The connection being replaced, for a custom edit or a sign-in again. */
	replacing?: Credential | undefined;
	/** The name a new account starts with: the provider's, numbered when it already has one. */
	label?: string | undefined;
	onConnected(made: Credential): void;
	onCancel(): void;
}) {
	const initial = provider.credentialKinds.length === 1 ? (provider.credentialKinds[0] ?? null) : null;
	const [flow, step] = useReducer(signInStep, initial === null || initial === "oauth" ? { k: "choosing" } : { k: "entering", kind: initial });
	const busy = signInBusy(flow);
	const login = flow.k === "waiting" ? flow.login : null;
	const form = formOf(flow);
	const refusal: Refused | null = flow.k === "refused" ? flow.refusal : null;
	const back = restingAfter(initial);
	const attempt = useRef(0);
	const finished = useRef(false);

	/* The credential exists. Its models are read now, so the first picker
	 * that opens has them; a provider whose list needs an account, or a
	 * local server, is read when its page opens instead. */
	const finish = async (created: Credential) => {
		if (finished.current) return;
		finished.current = true;
		// Another account of a provider already connected starts on its own name.
		const made =
			label !== undefined && label !== created.label
				? await wire.command("credential.rename", { id: created.id, label }).catch(() => created)
				: created;
		if (provider.modelDiscovery && made.providerId !== "ollama" && made.providerId !== "github-copilot") {
			await wire.command("credential.refresh_models", { providerId: made.providerId }).catch(() => {});
		}
		onConnected(made);
		// Still mounted: the form is free again rather than busy for good.
		finished.current = false;
		step({ type: "reset" });
	};

	const signIn = async () => {
		if (busy) return;
		step({ type: "begin" });
		const mine = ++attempt.current;
		try {
			const prompt = await wire.command("credential.login", { providerId: provider.id });
			if (mine !== attempt.current) {
				await wire.command("credential.login_cancel", { loginId: prompt.loginId });
				return;
			}
			step({ type: "prompted", login: prompt });
		} catch (error) {
			if (mine !== attempt.current) return;
			step({ type: "refused", refusal: refused(SIGN_IN_FAILED, error), back });
		}
	};

	// A provider with one way in, and that way a sign-in, starts it at once.
	// biome-ignore lint/correctness/useExhaustiveDependencies: a sign-in starts once, when the form opens.
	useEffect(() => {
		if (initial === "oauth") void signIn();
		return () => {
			attempt.current += 1;
		};
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: one poll per login, since restarting it cancels the sign-in; the handlers it calls hold only setters and keyed props.
	useEffect(() => {
		if (login === null) return;
		let cancelled = false;
		let ended = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const tick = () => {
			void wire
				.command("credential.login_status", { loginId: login.loginId })
				.then((status) => {
					if (cancelled) return;
					if (status.state === "done") {
						ended = true;
						if (status.credential) {
							step({ type: "saving", form: { k: "choosing" } });
							void finish(status.credential);
						} else {
							step({ type: "reset" });
							onCancel();
						}
						return;
					}
					if (status.state === "failed") {
						ended = true;
						step({ type: "refused", refusal: status.error === undefined ? { message: SIGN_IN_FAILED } : refused(SIGN_IN_FAILED, status.error), back });
						return;
					}
					timer = setTimeout(tick, 2000);
				})
				.catch((error: Error) => {
					if (cancelled) return;
					// The poll gave up: leave the waiting state like the failed branch does.
					step({ type: "refused", refusal: refused(SIGN_IN_FAILED, error), back });
				});
		};
		timer = setTimeout(tick, 2000);
		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
			// A login that finished or failed has nothing left to cancel.
			if (!ended) void wire.command("credential.login_cancel", { loginId: login.loginId }).catch(() => {});
		};
	}, [login?.loginId]);

	const cancelLogin = async () => {
		attempt.current += 1;
		try {
			if (login) await wire.command("credential.login_cancel", { loginId: login.loginId });
		} catch {
			// Cancelled is cancelled; the core drops the login when the socket does.
		} finally {
			step({ type: "reset" });
			onCancel();
		}
	};

	const saveKey = async (secret: string) => {
		if (busy) return;
		step({ type: "saving", form: { k: "entering", kind: "api_key" } });
		try {
			await finish(await wire.command("credential.create", { providerId: provider.id, label: provider.name, secret }));
		} catch (error) {
			step({ type: "refused", refusal: refused("Connection failed", error), back: { k: "entering", kind: "api_key" } });
		}
	};

	const connectLocal = async (baseUrl: string) => {
		if (busy) return;
		step({ type: "saving", form: { k: "entering", kind: "local" } });
		try {
			await finish(await wire.command("credential.connect_local", { baseUrl }));
		} catch (error) {
			step({ type: "refused", refusal: refused("Connection failed", error), back: { k: "entering", kind: "local" } });
		}
	};

	if (provider.id === "openai-compatible") {
		return <CustomProviderForm key={replacing?.id ?? "new"} credential={replacing} onSaved={(made) => void finish(made)} onCancel={onCancel} />;
	}

	return (
		<>
			{form.k === "choosing" && (flow.k === "choosing" || flow.k === "refused") && (
				<section>
					<h3 className="group-title">Connect {provider.name}</h3>
					<div className="grouped">
						{provider.credentialKinds.map((kind) => (
							<button
								key={kind}
								type="button"
								className="group-row group-row-choice w-full text-left"
								disabled={busy}
								onClick={() => (kind === "oauth" ? void signIn() : step({ type: "enter", kind }))}
							>
								<span className="group-row-text">
									{provider.id === "xai" && kind === "oauth" ? "Sign in with SuperGrok or X Premium+" : connectionMethod(kind)}
								</span>
								<ChevronRightIcon />
							</button>
						))}
						<div className="group-row justify-end">
							<button type="button" className="control btn-quiet" onClick={onCancel}>
								Cancel
							</button>
						</div>
					</div>
				</section>
			)}
			{form.k === "entering" && form.kind === "local" && <LocalProviderForm busy={busy} onSave={(url) => void connectLocal(url)} onCancel={onCancel} />}
			{form.k === "entering" && form.kind === "api_key" && <KeyForm provider={provider} busy={busy} onSave={(secret) => void saveKey(secret)} onCancel={onCancel} />}
			{(flow.k === "preparing" || flow.k === "waiting" || (flow.k === "finishing" && form.k === "choosing")) && (
				<section>
					<h3 className="group-title">Sign in to {provider.name}</h3>
					<div className="grouped">
						{login === null ? (
							<p className="group-row text-sm text-ink-3">{flow.k === "finishing" ? "Finishing sign-in…" : "Preparing sign-in…"}</p>
						) : (
							<>
								{login.userCode !== "" && (
									<div className="group-row justify-between">
										<span className="selectable font-mono text-xl tracking-wide">{login.userCode}</span>
										<CopyCode code={login.userCode} />
									</div>
								)}
								<div className="group-row">
									<button type="button" className="text-sm text-ink-2 underline" onClick={() => void openSignIn(login)}>
										Open {provider.name} sign-in
									</button>
								</div>
								<p className="group-row text-sm text-ink-3">Waiting for you to sign in…</p>
							</>
						)}
						<div className="group-row justify-end">
							<button type="button" className="control btn-quiet" onClick={() => void cancelLogin()}>
								Cancel
							</button>
						</div>
					</div>
					<p className="group-hint">{login?.userCode ? "Opening the page copies the code. Paste it there." : "Finish signing in with your browser, then return here."}</p>
				</section>
			)}
			{refusal !== null && <Refusal {...refusal} />}
		</>
	);
}

/** The sign-in page, with its code already on the clipboard to paste there. */
async function openSignIn(login: { userCode: string; verificationUri: string }) {
	if (login.userCode !== "") await writeClipboard(login.userCode).catch(() => {});
	await openLink(login.verificationUri);
}

/** A sign-in code's Copy, so it is pasted on the sign-in page rather than
 * typed: it says Copied for a moment once the clipboard has it. */
function CopyCode({ code }: { code: string }) {
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const timer = window.setTimeout(() => setCopied(false), 1500);
		return () => window.clearTimeout(timer);
	}, [copied]);
	return (
		<button type="button" className="control btn btn-sm gap-1.5" title="Copy code" aria-label="Copy sign-in code" onClick={() => void writeClipboard(code).then(() => setCopied(true))}>
			{copied ? <CheckIcon /> : <CopyIcon />}
			{copied ? "Copied" : "Copy"}
		</button>
	);
}

/** One provider not yet connected, as the add list offers it: its name and
 * the ways in. Pressing it begins the connection. */
export function ProviderRow({ provider, disabled, onPick }: { provider: Provider; disabled?: boolean; onPick(): void }) {
	return (
		<button type="button" className="group-row group-row-choice w-full text-left" disabled={disabled} onClick={onPick}>
			<span className="group-row-text">
				<span className="group-row-title">{provider.name}</span>
			</span>
			<span className="text-sm text-ink-3">{jobsOf(provider) ?? provider.credentialKinds.map(connectionMethod).join(" or ")}</span>
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</button>
	);
}

const JOB_NAMES: Record<ProviderJob, string> = { chat: "Chat", images: "Images", voice: "Voice" };

/**
 * What a provider does once connected, as the add list says it, so the ones
 * that do everything stand out; undefined from a desk too old to say, which
 * the list shows how it connects instead.
 */
export function jobsOf(provider: Provider): string | undefined {
	return provider.does === undefined || provider.does.length === 0 ? undefined : provider.does.map((job) => JOB_NAMES[job]).join(" · ");
}

/** How a provider is connected, as the add list says it. */
export function connectionMethod(kind: CredentialKind): string {
	return kind === "oauth" ? "Sign in" : kind === "local" ? "Server URL" : "API key";
}

function LocalProviderForm({ busy, onSave, onCancel }: { busy: boolean; onSave(url: string): void; onCancel(): void }) {
	const [url, setUrl] = useState("http://localhost:11434");
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (url.trim()) onSave(url.trim());
			}}
		>
			<h3 className="group-title">Connect Ollama Local</h3>
			<div className="grouped">
				<div className="group-row">
					<label htmlFor="ollama-url" className="w-24 shrink-0 text-sm text-ink-2">
						Server URL
					</label>
					<input
						id="ollama-url"
						type="url"
						className="field flex-1 font-mono text-sm"
						value={url}
						onChange={(event) => setUrl(event.target.value)}
						disabled={busy}
						spellCheck={false}
						autoComplete="off"
					/>
				</div>
				<div className="group-row justify-end">
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
						Cancel
					</button>
					<button type="submit" className="control btn-primary" disabled={busy || !url.trim()}>
						{busy ? "Connecting…" : "Connect"}
					</button>
				</div>
			</div>
			<p className="group-hint">Start Ollama first. Hotline discovers the models installed on this server. Cloud models available through your Ollama sign-in work here too.</p>
		</form>
	);
}

/** The key field, in place, for the provider just chosen. */
function KeyForm({
	provider,
	busy,
	onSave,
	onCancel,
}: {
	provider: Provider;
	busy: boolean;
	onSave(secret: string): void;
	onCancel(): void;
}) {
	const [secret, setSecret] = useState("");
	const field = useRef<HTMLInputElement>(null);
	useEffect(() => field.current?.focus(), []);
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (secret.trim() !== "") onSave(secret.trim());
			}}
		>
			<h3 className="group-title">Add {provider.name}</h3>
			<div className="grouped">
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="key-secret">
						API key
					</label>
					<input
						ref={field}
						id="key-secret"
						type="password"
						className="field flex-1 font-mono text-sm"
						spellCheck={false}
						autoComplete="off"
						value={secret}
						onChange={(event) => setSecret(event.target.value)}
					/>
				</div>
				<div className="group-row">
					<span className="min-w-0 flex-1">
						{provider.doc !== undefined && (
							<button type="button" className="nt-fold-action" onClick={() => void openLink(provider.doc ?? "")}>
								Where to get a key ↗
							</button>
						)}
					</span>
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
						Cancel
					</button>
					<button type="submit" className="control btn-primary" disabled={busy || secret.trim() === ""}>
						{busy ? "Saving…" : "Save key"}
					</button>
				</div>
			</div>
		</form>
	);
}
