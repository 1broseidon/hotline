import { useEffect, useRef, useState } from "react";
import type { Credential, CredentialKind, LoginPrompt, Provider } from "../generated/contract";
import { ChevronRightIcon } from "../icons";
import { openLink } from "../native";
import { Refusal } from "../ui/Refusal";
import { wire } from "../wire";
import { CustomProviderForm } from "./CustomProviderForm";

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
	onConnected,
	onCancel,
}: {
	provider: Provider;
	/** The connection being replaced, for a custom edit or a sign-in again. */
	replacing?: Credential | undefined;
	onConnected(made: Credential): void;
	onCancel(): void;
}) {
	const initial = provider.credentialKinds.length === 1 ? (provider.credentialKinds[0] ?? null) : null;
	const [method, setMethod] = useState<CredentialKind | null>(initial);
	const [login, setLogin] = useState<LoginPrompt | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const attempt = useRef(0);
	const finished = useRef(false);

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
			const prompt = await wire.command("credential.login", { providerId: provider.id });
			if (mine !== attempt.current) {
				await wire.command("credential.login_cancel", { loginId: prompt.loginId });
				return;
			}
			setLogin(prompt);
		} catch (error) {
			if (mine !== attempt.current) return;
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
			setMethod(initial === "oauth" ? null : initial);
		}
	};

	// A provider with one way in, and that way a sign-in, starts it at once.
	useEffect(() => {
		if (initial === "oauth") void signIn();
		return () => {
			attempt.current += 1;
		};
	}, []);

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
						setRefusal(status.error ?? "Sign-in failed.");
						setLogin(null);
						setBusy(false);
						setMethod(initial === "oauth" ? null : initial);
						return;
					}
					timer = setTimeout(tick, 2000);
				})
				.catch((error: Error) => {
					if (cancelled) return;
					setRefusal(error.message);
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

	const saveKey = async (secret: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await finish(await wire.command("credential.create", { providerId: provider.id, label: provider.name, secret }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const connectLocal = async (baseUrl: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await finish(await wire.command("credential.connect_local", { baseUrl }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	if (provider.id === "openai-compatible") {
		return <CustomProviderForm key={replacing?.id ?? "new"} credential={replacing} onSaved={(made) => void finish(made)} onCancel={onCancel} />;
	}

	return (
		<>
			{method === null && (
				<section>
					<h3 className="group-title">Connect {provider.name}</h3>
					<div className="grouped">
						{provider.credentialKinds.map((kind) => (
							<button
								key={kind}
								type="button"
								className="group-row group-row-choice w-full text-left"
								disabled={busy}
								onClick={() => (kind === "oauth" ? void signIn() : setMethod(kind))}
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
			{method === "local" && <LocalProviderForm busy={busy} onSave={(url) => void connectLocal(url)} onCancel={onCancel} />}
			{method === "api_key" && <KeyForm provider={provider} busy={busy} onSave={(secret) => void saveKey(secret)} onCancel={onCancel} />}
			{method === "oauth" && (
				<section>
					<h3 className="group-title">Sign in to {provider.name}</h3>
					<div className="grouped">
						{login === null ? (
							<p className="group-row text-sm text-ink-3">Preparing sign-in…</p>
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
				</section>
			)}
			{refusal !== null && <Refusal message={refusal} />}
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
			<span className="text-sm text-ink-3">{provider.credentialKinds.map(connectionMethod).join(" or ")}</span>
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</button>
	);
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
				<div className="group-row justify-end">
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
						Cancel
					</button>
					<button type="submit" className="control btn-primary" disabled={busy || secret.trim() === ""}>
						{busy ? "Saving…" : "Save key"}
					</button>
				</div>
			</div>
			{provider.doc !== undefined && (
				<p className="group-hint">
					<button type="button" className="underline" onClick={() => void openLink(provider.doc ?? "")}>
						Where to get one
					</button>
				</p>
			)}
		</form>
	);
}
