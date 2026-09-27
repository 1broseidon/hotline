import { useEffect, useRef, useState } from "react";
import type { Terminal } from "@xterm/xterm";
import { openLink } from "../native";
import { wire } from "../wire";
import "@xterm/xterm/css/xterm.css";

export type AgentSignInAction = {
	harnessName: string;
	methods: { id: string; name: string; description?: string }[];
};

/** Login belongs to the harness. This terminal is private, transient UI, never
 * a draft, transcript, or model input. Unmounting cancels the owning job. */
export function AgentSignIn({ personaId, action, onRetry }: {
	personaId: string;
	action: AgentSignInAction;
	onRetry?: () => void;
}) {
	const [methodId, setMethodId] = useState(action.methods[0]?.id ?? "");
	const [attempt, setAttempt] = useState<string | null>(null);
	const [succeeded, setSucceeded] = useState(false);
	const [reviewed, setReviewed] = useState(false);
	if (succeeded) return (
		<div className="mt-3 text-sm" role="status">
			<p>Signed in. The teammate is ready for another message.</p>
			{onRetry && !reviewed && <>
				<p className="mt-1 text-ink-3">The failed turn may have already done some work. Review before sending it again.</p>
				<button className="control btn mt-2" onClick={() => { onRetry(); setReviewed(true); }}>Review failed message</button>
			</>}
		</div>
	);
	return (
		<div className="mt-3">
			{attempt === null ? <>
				{action.methods.length > 1 && <label className="mr-2 text-sm">
					Sign-in method
					<select className="control ml-2" value={methodId} onChange={(event) => setMethodId(event.target.value)}>
						{action.methods.map((method) => <option key={method.id} value={method.id}>{method.name}</option>)}
					</select>
				</label>}
				<button className="control btn" onClick={() => setAttempt(methodId)} disabled={!methodId}>Sign in</button>
				<p className="mt-1 text-xs text-ink-3">Opens {action.harnessName}’s own sign-in on this computer.</p>
			</> : <SignInAttempt personaId={personaId} methodId={attempt}
				onDone={() => { setAttempt(null); setSucceeded(true); }} onClose={() => setAttempt(null)} />}
		</div>
	);
}

function SignInAttempt({ personaId, methodId, onDone, onClose }: {
	personaId: string;
	methodId: string;
	onDone(): void;
	onClose(): void;
}) {
	const host = useRef<HTMLDivElement>(null);
	const callbacks = useRef({ onDone, onClose });
	callbacks.current = { onDone, onClose };
	const [error, setError] = useState<string | null>(null);
	const [running, setRunning] = useState(true);

	useEffect(() => {
		let disposed = false;
		let ended = false;
		let id: string | null = null;
		let terminal: Terminal | undefined;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let inputQueue = Promise.resolve();
		const cancel = () => {
			if (id !== null) void wire.command("agent.auth.cancel", { personaId, id }).catch(() => {});
		};
		const failed = (cause: unknown) => {
			if (ended) return;
			ended = true;
			clearTimeout(timer);
			cancel();
			if (disposed) return;
			setError(cause instanceof Error ? cause.message : String(cause));
			setRunning(false);
			terminal?.dispose();
			terminal = undefined;
		};
		const poll = async () => {
			if (disposed || ended || id === null) return;
			try {
				const status = await wire.command("agent.auth.poll", { personaId, id });
				if (disposed || ended) return;
				if (status.output) terminal?.write(status.output);
				if (status.state === "succeeded") {
					ended = true;
					id = null;
					callbacks.current.onDone();
				} else if (status.state === "failed") {
					failed(status.error ?? "Sign-in did not finish. You can try again.");
				} else {
					timer = setTimeout(() => { void poll(); }, 200);
				}
			} catch (cause) { failed(cause); }
		};
		void (async () => {
			try {
				const [{ Terminal }, { WebLinksAddon }] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-web-links")]);
				if (disposed || !host.current) return;
				const followLink = (_event: MouseEvent, url: string) => {
					// Terminal escape sequences cannot open local files or custom schemes.
					if (/^https?:\/\//i.test(url)) void openLink(url);
				};
				terminal = new Terminal({ cols: 100, rows: 30, scrollback: 100, fontSize: 12,
					screenReaderMode: true, allowProposedApi: false, linkHandler: { activate: followLink },
					theme: { background: "#151515", foreground: "#eeeeee" },
				});
				terminal.loadAddon(new WebLinksAddon(followLink));
				terminal.open(host.current);
				terminal.onData((input) => {
					// Ordered, dedicated input: no draft state, console, or local echo.
					inputQueue = inputQueue.then(async () => {
						if (disposed || ended || id === null) return;
						await wire.command("agent.auth.input", { personaId, id, input });
					}).catch(failed);
				});
				const started = await wire.command("agent.auth.start", { personaId, methodId });
				id = started.id;
				if (disposed) { cancel(); return; }
				terminal.focus();
				void poll();
			} catch (cause) { failed(cause); }
		})();
		return () => {
			disposed = true;
			clearTimeout(timer);
			cancel();
			terminal?.dispose();
		};
	}, [personaId, methodId]);

	return <div aria-label="Harness sign-in" className="rounded border border-line p-2">
		<p className="mb-2 text-xs text-ink-3">Complete the harness’s sign-in here or in the browser it opens. This is not sent to the conversation.</p>
		{error && <p role="alert" className="mb-2 text-sm text-danger">{error}</p>}
		<div ref={host} data-private-terminal onKeyDown={(event) => event.stopPropagation()} className="max-w-full overflow-x-auto" />
		<button className="control btn mt-2" onClick={() => callbacks.current.onClose()}>{running ? "Cancel sign-in" : "Close"}</button>
	</div>;
}
