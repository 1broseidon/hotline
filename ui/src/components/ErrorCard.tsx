import { AgentSignIn, type AgentSignInAction } from "./AgentSignIn";
import { WarningIcon } from "../icons";

export function errorDetails(text: string): { title: string; summary: string; details: string; context?: string; signIn?: AgentSignInAction } {
    const start = text.indexOf('{"hotlineFailure":');
    if (start !== -1) {
        try {
            const { hotlineFailure: failure } = JSON.parse(text.slice(start));
            if (typeof failure?.title === "string" && typeof failure?.summary === "string" && typeof failure?.details === "string") {
                const context = [
                    typeof failure.status === "number" ? `HTTP ${failure.status}` : null,
                    typeof failure.code === "string" ? failure.code : null,
                    typeof failure.retryAfterSeconds === "number" ? `Retry after ${failure.retryAfterSeconds}s` : null,
                ].filter(Boolean).join(" · ");
                const signIn = signInAction(failure);
                return { title: failure.title, summary: failure.summary, details: failure.details, context, ...(signIn ? { signIn } : {}) };
            }
        } catch { /* Older notices are plain text. */ }
    }
    let details = text;
    const json = text.indexOf("{");
    if (json !== -1) {
        try { details = text.slice(0, json) + JSON.stringify(JSON.parse(text.slice(json)), null, 2); } catch { /* Keep the original diagnostic. */ }
    }
    return { title: "Turn failed", summary: "The activity could not finish. View the reported error below.", details };
}

function signInAction(failure: { kind?: unknown; signIn?: unknown }): AgentSignInAction | undefined {
    if (failure.kind !== "agent_auth" || !failure.signIn || typeof failure.signIn !== "object") return;
    const action = failure.signIn as Partial<AgentSignInAction>;
    if (typeof action.harnessName !== "string" || !Array.isArray(action.methods)) return;
    const methods = action.methods.filter((method) => method && typeof method.id === "string" && method.id !== "" && typeof method.name === "string")
        .map((method) => ({ id: method.id, name: method.name, ...(typeof method.description === "string" ? { description: method.description } : {}) }));
    if (methods.length === 0) return;
    return { harnessName: action.harnessName, methods };
}

export function ErrorCard({ text, personaId, onRetry }: { text: string; personaId?: string; onRetry?: () => void }) {
    const error = errorDetails(text);
    return (
        <section className="my-2 min-w-0 max-w-full rounded-lg border border-line bg-raised p-3" aria-label={error.title}>
            <div className="flex items-center gap-2 text-danger">
                <WarningIcon className="shrink-0" />
                <span className="font-medium">{error.title}</span>
            </div>
            <p className="mt-1 text-sm text-ink-2">{error.summary}</p>
            {error.signIn && personaId && <AgentSignIn personaId={personaId} action={error.signIn} {...(onRetry ? { onRetry } : {})} />}
            <details className="mt-2 min-w-0">
                <summary className="cursor-pointer text-xs text-ink-3 focus-visible:outline focus-visible:outline-2">Error details</summary>
                {error.context && <p className="mt-2 break-words text-xs text-ink-3" style={{ overflowWrap: "anywhere" }}>{error.context}</p>}
                <pre className="selectable mt-2 max-h-80 max-w-full overflow-auto whitespace-pre-wrap break-words text-xs text-ink-2" style={{ overflowWrap: "anywhere" }}>{error.details}</pre>
            </details>
        </section>
    );
}
