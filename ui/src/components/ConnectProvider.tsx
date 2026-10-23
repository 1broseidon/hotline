import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import type { Credential, CredentialKind, LoginPrompt, Provider, ProviderJob } from "../generated/contract";
import { ChevronRightIcon } from "../icons";
import { openLink } from "../native";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { wire } from "../wire";
import { freeName } from "./accounts/words";
import { CustomProviderForm } from "./CustomProviderForm";

const SIGN_IN_FAILED = "Sign-in failed";

/**
 * Connecting one provider, in place: the method when it offers more than
 * one, then the key field, the server URL, the sign-in wait, or the custom
 * connection form. Settings › Providers and the welcome pane both render
 * this, so a key is pasted the same way on the first day as on any other.
 *
 * With `credentialId` it changes that account in place instead: a new key,
 * a new server, or a sign-in again. The account keeps its id, name and
 * teammates, so its provider's default never moves.
 *
 * The credential is handed up once it exists and its models are read; what
 * to do with it — list it, open its page, move on to the first teammate —
 * is the caller's. Cancelling a sign-in cancels it on the core too.
 */
export function ConnectProvider({
	provider,
	kinds = provider.credentialKinds,
	credentialId,
	taken = [],
	onConnected,
	onCancel,
}: {
	provider: Provider;
	/** The ways in offered; a provider's page offers one section's. */
	kinds?: CredentialKind[];
	credentialId?: string | undefined;
	/** Its accounts' names, so a blank name takes the next free one. */
	taken?: string[];
	onConnected(made: Credential): void;
	onCancel(): void;
}) {
	const initial = kinds.length === 1 ? (kinds[0] ?? null) : null;
	const changing = credentialId !== undefined;
	const [method, setMethod] = useState<CredentialKind | null>(initial);
	const [name, setName] = useState("");
	const [login, setLogin] = useState<LoginPrompt | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<Refused | null>(null);
	const attempt = useRef(0);
	const finished = useRef(false);
	const placeholder = freeName(provider.name, taken);
	const label = name.trim() || placeholder;
	const naming = changing ? undefined : <NameRow value={name} placeholder={placeholder} busy={busy} onChange={setName} />;

	/* The credential exists. Its models are read now, so the first picker
	 * that opens has them; a provider whose list needs an account, or a
	 * local server, is read when its page opens instead. */
	const finish = async (made: Credential) => {
		if (finished.current) return;
		finished.current = true;
		if (provider.modelDiscovery && made.providerId !== "ollama" && made.providerId !== "github-copilot") {
			await wire.command("credential.refresh_models", { providerId: made.providerId }).catch(() => {});
		}
		onConnected(made);
	};

	const signIn = async () => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		setMethod("oauth");
		const mine = ++attempt.current;
		try {
			const prompt = await wire.command(
				"credential.login",
				credentialId !== undefined ? { providerId: provider.id, id: credentialId } : { providerId: provider.id, label },
			);
			if (mine !== attempt.current) {
				await wire.command("credential.login_cancel", { loginId: prompt.loginId });
				return;
			}
			setLogin(prompt);
		} catch (error) {
			if (mine !== attempt.current) return;
			setRefusal(refused(SIGN_IN_FAILED, error));
			setBusy(false);
		}
	};

	// A sign-in again starts at once: the account it signs in has its name already.
	// biome-ignore lint/correctness/useExhaustiveDependencies: a sign-in starts once, when the form opens.
	useEffect(() => {
		if (changing && initial === "oauth") void signIn();
		return () => {
			attempt.current += 1;
		};
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: one poll per login, since restarting it cancels the sign-in; the handlers it calls hold only setters and keyed props.
	useEffect(() => {
		if (login === null) return;
		let cancelled = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const tick = () => {
			void wire
				.command("credential.login_status", { loginId: login.loginId })
				.then((status) => {
					if (cancelled) return;
					if (status.state === "done") {
						setLogin(null);
						if (status.credential) {
							void finish(status.credential);
						} else {
							setBusy(false);
							onCancel();
						}
						return;
					}
					if (status.state === "failed") {
						setRefusal(status.error === undefined ? { message: SIGN_IN_FAILED } : refused(SIGN_IN_FAILED, status.error));
						setLogin(null);
						setBusy(false);
						return;
					}
					timer = setTimeout(tick, 2000);
				})
				.catch((error: Error) => {
					if (cancelled) return;
					setRefusal(refused(SIGN_IN_FAILED, error));
					setLogin(null);
					setBusy(false);
				});
		};
		timer = setTimeout(tick, 2000);
		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
			void wire.command("credential.login_cancel", { loginId: login.loginId }).catch(() => {});
		};
	}, [login]);

	const cancelLogin = async () => {
		attempt.current += 1;
		try {
			if (login) await wire.command("credential.login_cancel", { loginId: login.loginId });
		} catch {
			// Cancelled is cancelled; the core drops the login when the socket does.
		} finally {
			setLogin(null);
			setBusy(false);
			onCancel();
		}
	};

	const save = async (work: () => Promise<Credential>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await finish(await work());
		} catch (error) {
			setRefusal(refused("Connection failed", error));
			setBusy(false);
		}
	};

	const saveKey = (secret: string) =>
		save(() =>
			credentialId !== undefined
				? wire.command("credential.update", { id: credentialId, secret })
				: wire.command("credential.create", { providerId: provider.id, label, secret }),
		);

	const connectLocal = (baseUrl: string) =>
		save(() =>
			credentialId !== undefined
				? wire.command("credential.update", { id: credentialId, baseUrl })
				: wire.command("credential.connect_local", { baseUrl, label }),
		);

	if (provider.id === "openai-compatible") {
		return <CustomProviderForm credential={undefined} onSaved={(made) => void finish(made)} onCancel={onCancel} />;
	}

	return (
		<>
			{method === null && (
				<section>
					<h3 className="group-title">Connect {provider.name}</h3>
					<div className="grouped">
						{kinds.map((kind) => (
							<button
								key={kind}
								type="button"
								className="group-row group-row-choice w-full text-left"
								disabled={busy}
								onClick={() => setMethod(kind)}
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
			{method === "local" && (
				<LocalProviderForm title={changing ? "Change server" : "Connect Ollama Local"} naming={naming} busy={busy} onSave={(url) => void connectLocal(url)} onCancel={onCancel} />
			)}
			{method === "api_key" && (
				<KeyForm title={changing ? "Change key" : `Add ${provider.name}`} naming={naming} doc={provider.doc} busy={busy} onSave={(secret) => void saveKey(secret)} onCancel={onCancel} />
			)}
			{method === "oauth" && (
				<form
					onSubmit={(event) => {
						event.preventDefault();
						void signIn();
					}}
				>
					<h3 className="group-title">
						Sign in to {provider.name}
						{changing ? " again" : ""}
					</h3>
					<div className="grouped">
						{login !== null ? (
							<>
								{login.userCode !== "" && (
									<div className="group-row">
										<span className="selectable font-mono text-xl tracking-wide">{login.userCode}</span>
									</div>
								)}
								<div className="group-row">
									<button type="button" className="text-sm text-ink-2 underline" onClick={() => void openLink(login.verificationUri)}>
										Open {provider.name} sign-in
									</button>
								</div>
								<p className="group-row text-sm text-ink-3">Waiting for you to sign in…</p>
							</>
						) : busy ? (
							<p className="group-row text-sm text-ink-3">Preparing sign-in…</p>
						) : (
							naming
						)}
						<div className="group-row justify-end">
							<button type="button" className="control btn-quiet" onClick={() => void cancelLogin()}>
								Cancel
							</button>
							{!busy && (
								<button type="submit" className="control btn-primary">
									Sign in
								</button>
							)}
						</div>
					</div>
					{busy && <p className="group-hint">{login?.userCode ? "Enter the code on that page." : "Finish signing in with your browser, then return here."}</p>}
				</form>
			)}
			{refusal !== null && <Refusal {...refusal} />}
		</>
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

function NameRow({ value, placeholder, busy, onChange }: { value: string; placeholder: string; busy: boolean; onChange(value: string): void }) {
	const id = useId();
	return (
		<div className="group-row">
			<label htmlFor={id} className="w-24 shrink-0 text-sm text-ink-2">
				Name
			</label>
			<input
				id={id}
				className="field flex-1"
				placeholder={placeholder}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				disabled={busy}
				spellCheck={false}
				autoComplete="off"
			/>
		</div>
	);
}

function LocalProviderForm({
	title,
	naming,
	busy,
	onSave,
	onCancel,
}: {
	title: string;
	naming: ReactNode;
	busy: boolean;
	onSave(url: string): void;
	onCancel(): void;
}) {
	const [url, setUrl] = useState("http://localhost:11434");
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (url.trim()) onSave(url.trim());
			}}
		>
			<h3 className="group-title">{title}</h3>
			<div className="grouped">
				{naming}
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
	title,
	naming,
	doc,
	busy,
	onSave,
	onCancel,
}: {
	title: string;
	naming: ReactNode;
	doc: string | undefined;
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
			<h3 className="group-title">{title}</h3>
			<div className="grouped">
				{naming}
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
						{doc !== undefined && (
							<button type="button" className="nt-fold-action" onClick={() => void openLink(doc)}>
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
