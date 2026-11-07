import { useEffect, useReducer, useRef, useState } from "react";
import type { Credential, CredentialKind, Provider, ProviderJob } from "../generated/contract";
import { ChevronRightIcon } from "../icons";
import { openLink } from "../native";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { onServer } from "../serverFiles";
import { formOf, restingAfter, signInBusy, signInStep } from "../signIn";
import { wire } from "../wire";
import { CustomProviderForm } from "./CustomProviderForm";

const SIGN_IN_FAILED = "Sign-in failed";

/** ChatGPT signs in through the browser as Hotline, or with a device code as before. */
const CHATGPT = "openai-codex";

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
	// ChatGPT offers two ways in, both sign-ins, so it starts on the choice.
	const initial = provider.credentialKinds.length === 1 && provider.id !== CHATGPT ? (provider.credentialKinds[0] ?? null) : null;
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

	const signIn = async (browser = false) => {
		if (busy) return;
		step({ type: "begin" });
		const mine = ++attempt.current;
		try {
			const prompt = await wire.command("credential.login", browser ? { providerId: provider.id, browser } : { providerId: provider.id });
			if (mine !== attempt.current) {
				await wire.command("credential.login_cancel", { loginId: prompt.loginId });
				return;
			}
			step({ type: "prompted", login: prompt });
			// Sign in with ChatGPT opens the browser itself; the link below opens it again.
			if (browser) void openLink(prompt.verificationUri);
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
						{provider.id === CHATGPT && (
							<>
								<button type="button" className="group-row group-row-choice w-full text-left" disabled={busy} onClick={() => void signIn(true)}>
									<span className="group-row-text">Sign in with ChatGPT</span>
									<ChevronRightIcon />
								</button>
								<button type="button" className="group-row group-row-choice w-full text-left" disabled={busy} onClick={() => void signIn()}>
									<span className="group-row-text">Use a device code</span>
									<ChevronRightIcon />
								</button>
							</>
						)}
						{provider.id !== CHATGPT && provider.credentialKinds.map((kind) => (
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
						)}
						<div className="group-row justify-end">
							<button type="button" className="control btn-quiet" onClick={() => void cancelLogin()}>
								Cancel
							</button>
						</div>
					</div>
					<p className="group-hint">{login?.userCode ? "Enter the code on that page." : "Finish signing in with your browser, then return here."}</p>
					{login?.redirectUri !== undefined && <LoginPasteBack loginId={login.loginId} redirectUri={login.redirectUri} />}
				</section>
			)}
			{refusal !== null && <Refusal {...refusal} />}
		</>
	);
}

/**
 * The fallback for a browser sign-in whose browser can't reach the desk:
 * one on another computer than a server desk, or one whose page never came
 * back. The page it ends on won't load, and its address holds what the
 * desk is waiting for. A wrong address is refused and the sign-in waits on;
 * the right one finishes it, and the poll above sees it done.
 */
function LoginPasteBack({ loginId, redirectUri }: { loginId: string; redirectUri: string }) {
	const [address, setAddress] = useState("");
	const [busy, setBusy] = useState(false);
	const [refused, setRefused] = useState<string | null>(null);

	const finish = async () => {
		setBusy(true);
		setRefused(null);
		try {
			await wire.command("credential.login_redirect", { loginId, redirectUrl: address.trim() });
			setAddress("");
		} catch (error) {
			setRefused(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<>
			<p className="group-hint">
				{onServer()
					? "Approve in your browser, then paste the address of the page you land on (it won't load, and that's expected)."
					: "If your browser doesn't come back to Hotline, paste the address of the page it ends on."}
			</p>
			<label className="label mt-4" htmlFor="login-callback">
				Page address
			</label>
			<textarea
				id="login-callback"
				className="field h-20 w-full font-mono text-sm"
				placeholder={`${redirectUri}?…`}
				value={address}
				spellCheck={false}
				onChange={(event) => setAddress(event.target.value)}
			/>
			<div className="mt-3 flex justify-end">
				<button type="button" className="control btn-primary" disabled={busy || address.trim() === ""} onClick={() => void finish()}>
					{busy ? "Finishing…" : "Finish sign-in"}
				</button>
			</div>
			{refused !== null && <Refusal message="That address didn't finish the sign-in." detail={refused} />}
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
