import { useEffect, useState } from "react";
import type { ConfigChoice } from "../generated/contract";
import { accountName, agentOf, backendName, plainModel } from "../accounts";
import { wire } from "../wire";
import { AgentSignIn, type AgentSignInAction } from "./AgentSignIn";
import { WarningIcon } from "../icons";
import { Refusal, refused, type Refused } from "../ui/Refusal";

/**
 * The failures a second try can get past: the network, the provider having
 * a bad moment or turning a working sign-in away, a limit that has since
 * lifted, a harness that fell over, a keychain the person has since unlocked.
 * A refused key, a spent quota or a request the model will not take fails
 * the same way again, so it is not offered.
 */
const WORTH_ANOTHER_TRY = new Set(["transport", "provider", "provider_refused", "rate_limit", "acp", "keychain", "unknown"]);

/**
 * The failures another account may get past: one that is gone, and one that
 * has spent what its plan allows. The same account fails the same way again.
 */
const ANOTHER_ACCOUNT = new Set(["account_gone", "quota", "usage_limit"]);

export function errorDetails(text: string): {
    title: string;
    summary: string;
    details: string;
    context?: string;
    signIn?: AgentSignInAction;
    kind?: string;
    model?: string;
    backend?: string;
    resetsAt?: number;
} {
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
                return {
                    title: failure.title,
                    summary: failure.summary,
                    details: failure.details,
                    context,
                    ...(signIn ? { signIn } : {}),
                    ...(typeof failure.kind === "string" ? { kind: failure.kind } : {}),
                    ...(typeof failure.model === "string" ? { model: failure.model } : {}),
                    ...(typeof failure.backend === "string" ? { backend: failure.backend } : {}),
                    ...(typeof failure.resetsAt === "number" ? { resetsAt: failure.resetsAt } : {}),
                };
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

export function ErrorCard({
    text,
    personaId,
    onRetry,
    onRunAgain,
    onProviders,
}: {
    text: string;
    personaId?: string;
    onRetry?: () => void;
    /** Runs the failed turn again; offered only on the latest one. */
    onRunAgain?: () => Promise<unknown>;
    /** Opens Settings › Providers, the fix for Hotline Agent with no provider. */
    onProviders?: () => void;
}) {
    const error = errorDetails(text);
    const [again, setAgain] = useState<"idle" | "trying" | Refused>("idle");
    const offerAgain = onRunAgain !== undefined && error.kind !== undefined && WORTH_ANOTHER_TRY.has(error.kind);
    return (
        <section className="my-2 min-w-0 max-w-full rounded-lg border border-line bg-raised p-3" aria-label={error.title}>
            <div className="flex items-center gap-2 text-danger">
                <WarningIcon className="shrink-0" />
                <span className="font-medium">{error.title}</span>
            </div>
            <p className="mt-1 text-sm text-ink-2">{error.summary}</p>
            {error.resetsAt !== undefined && <p className="mt-1 text-sm text-ink-3">Resets {resetTime(error.resetsAt)}.</p>}
            {error.kind === "no_provider" && onProviders && (
                <div className="mt-2">
                    <button type="button" className="control btn btn-sm" onClick={onProviders}>
                        Settings › Providers
                    </button>
                </div>
            )}
            {ANOTHER_ACCOUNT.has(error.kind ?? "") && (error.model !== undefined || error.backend !== undefined) && personaId && (
                <OtherAccounts
                    personaId={personaId}
                    spent={error.kind !== "account_gone"}
                    {...(error.model !== undefined ? { model: error.model } : {})}
                    {...(error.backend !== undefined ? { backend: error.backend } : {})}
                    {...(onRunAgain ? { onRunAgain } : {})}
                />
            )}
            {error.signIn && personaId && <AgentSignIn personaId={personaId} action={error.signIn} {...(onRetry ? { onRetry } : {})} />}
            {offerAgain && (
                <div className="mt-2 flex items-center gap-3">
                    <button
                        type="button"
                        className="control btn btn-sm"
                        disabled={again === "trying"}
                        onClick={() => {
                            setAgain("trying");
                            onRunAgain()
                                .then(() => setAgain("idle"))
                                .catch((reason: unknown) => setAgain(refused("Couldn't try again", reason)));
                        }}
                    >
                        {again === "trying" ? "Trying again…" : "Try again"}
                    </button>
                </div>
            )}
            {offerAgain && typeof again === "object" && <Refusal {...again} />}
            <details className="mt-2 min-w-0">
                <summary className="cursor-pointer text-xs text-ink-3 focus-visible:outline focus-visible:outline-2">Error details</summary>
                {error.context && <p className="mt-2 break-words text-xs text-ink-3" style={{ overflowWrap: "anywhere" }}>{error.context}</p>}
                <pre className="selectable mt-2 max-h-80 max-w-full overflow-auto whitespace-pre-wrap break-words text-xs text-ink-2" style={{ overflowWrap: "anywhere" }}>{error.details}</pre>
            </details>
        </section>
    );
}

/** When a limit lifts, as a time of day, with the day when it is not today. */
function resetTime(at: number): string {
    const when = new Date(at);
    const time = when.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    if (when.toDateString() === new Date().toDateString()) return `at ${time}`;
    return `${when.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })} at ${time}`;
}

/**
 * A teammate whose account is gone or spent: the other accounts that can
 * take its turn, one button each. On Hotline Agent they are the accounts
 * serving the same model, and picking one is a model change; on an agent
 * that keeps its own sign-in they are that agent's other accounts, and
 * picking one moves the teammate there with its model. Either way the
 * failed turn runs again, and nothing moves it without a press.
 */
function OtherAccounts({
    personaId,
    model,
    backend,
    spent,
    onRunAgain,
}: {
    personaId: string;
    model?: string;
    /** The agent the teammate ran, account and all, when it is not Hotline Agent. */
    backend?: string;
    /** The account is still there, and the one that failed is not offered. */
    spent: boolean;
    onRunAgain?: () => Promise<unknown>;
}) {
    const [others, setOthers] = useState<{ id: string; name: string }[] | null>(null);
    const [state, setState] = useState<"idle" | "moving" | Refused>("idle");
    useEffect(() => {
        let live = true;
        const found =
            backend !== undefined
                ? wire.command("backends.list", {}).then((backends) =>
                      backends
                          .filter((one) => one.id !== backend && agentOf(one.id) === agentOf(backend) && one.unavailable === undefined)
                          .map((one) => ({ id: one.id, name: backendName(one) })),
                  )
                : wire.command("models.list", {}).then((choices) =>
                      othersServing(choices, model ?? "", spent).map((choice) => ({
                          id: choice.id,
                          name: choice.account ? accountName(choice.group ?? "", choice.account.label, choice.account.shared) : (choice.group ?? choice.name),
                      })),
                  );
        void found.then(
            (list) => live && setOthers(list),
            () => live && setOthers([]),
        );
        return () => {
            live = false;
        };
    }, [model, backend, spent]);
    if (others === null) return null;
    if (others.length === 0)
        return (
            <p className="mt-2 text-sm text-ink-3">
                {backend !== undefined ? "No other account of this agent is ready." : "No other account serves this model."} Add one in Settings › Providers.
            </p>
        );
    const move = (id: string) =>
        backend !== undefined
            ? wire.command("session.set_backend", { personaId, backendId: id, ...(model !== undefined ? { modelId: model } : {}) })
            : wire.command("session.set_model", { personaId, modelId: id });
    return (
        <div className="mt-2 flex flex-wrap items-center gap-2">
            {others.map((one) => (
                <button
                    key={one.id}
                    type="button"
                    className="control btn btn-sm"
                    disabled={state === "moving"}
                    onClick={() => {
                        setState("moving");
                        void move(one.id)
                            .then(() => onRunAgain?.())
                            .then(() => setState("idle"))
                            .catch((reason: unknown) => setState(refused("Couldn't move it", reason)));
                    }}
                >
                    Use {one.name}
                </button>
            ))}
            {typeof state === "object" && <Refusal {...state} />}
        </div>
    );
}

/**
 * The accounts serving a model. A spent one is left out: the model id names
 * it, or, plain, ran on the provider's default, the first account listing it.
 */
function othersServing(choices: ConfigChoice[], model: string, spent: boolean): ConfigChoice[] {
    const serving = choices.filter((choice) => plainModel(choice.id) === plainModel(model));
    if (!spent) return serving;
    const failed = serving.find((choice) => choice.id === model) ?? (model === plainModel(model) ? serving[0] : undefined);
    return serving.filter((choice) => choice !== failed);
}
