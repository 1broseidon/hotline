import { useState } from "react";
import { AccountFailure, type AccountOffer } from "./accounts/AccountFailure";
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
const WORTH_ANOTHER_TRY = new Set(["transport", "provider", "provider_refused", "rate_limit", "acp", "keychain", "usage_limit", "unknown"]);

/** The limits whose wait is hours, not seconds, so the card says it in words. */
const RESETS = new Set(["usage_limit", "quota"]);

export type FailureDetails = {
    title: string;
    summary: string;
    details: string;
    context?: string;
    signIn?: AgentSignInAction;
    kind?: string;
    accountId?: string;
    code?: string;
    retryAfterSeconds?: number;
};

export function errorDetails(text: string): FailureDetails {
    const start = text.indexOf('{"hotlineFailure":');
    if (start !== -1) {
        try {
            const { hotlineFailure: failure } = JSON.parse(text.slice(start));
            if (typeof failure?.title === "string" && typeof failure?.summary === "string" && typeof failure?.details === "string") {
                const context = [
                    typeof failure.status === "number" ? `HTTP ${failure.status}` : null,
                    typeof failure.code === "string" ? failure.code : null,
                    typeof failure.retryAfterSeconds === "number" && !RESETS.has(failure.kind) ? `Retry after ${failure.retryAfterSeconds}s` : null,
                ].filter(Boolean).join(" · ");
                const signIn = signInAction(failure);
                return {
                    title: failure.title,
                    summary: failure.summary,
                    details: failure.details,
                    context,
                    ...(signIn ? { signIn } : {}),
                    ...(typeof failure.kind === "string" ? { kind: failure.kind } : {}),
                    ...(typeof failure.accountId === "string" ? { accountId: failure.accountId } : {}),
                    ...(typeof failure.code === "string" ? { code: failure.code } : {}),
                    ...(typeof failure.retryAfterSeconds === "number" ? { retryAfterSeconds: failure.retryAfterSeconds } : {}),
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

// A sign-in missing on an account never offers the harness's own methods: they would sign in This computer, not the account.
function aboutAccount(error: FailureDetails): boolean {
    if (error.kind === "usage_limit" || error.kind === "account_unavailable") return true;
    return error.accountId !== undefined && (error.kind === "agent_auth" || error.kind === "quota" || error.kind === "auth");
}

function about(amount: number, unit: "minute" | "hour" | "day"): string {
    if (amount === 1) return unit === "hour" ? "an hour" : `a ${unit}`;
    return `${amount} ${unit}s`;
}

export function resetWords(seconds: number): string {
    if (seconds < 60) return "Resets in under a minute.";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `Resets in about ${about(minutes, "minute")}.`;
    const hours = Math.round(seconds / 3600);
    if (hours < 24) return `Resets in about ${about(hours, "hour")}.`;
    return `Resets in about ${about(Math.round(seconds / 86_400), "day")}.`;
}

// The wait is counted from when the notice was written, so a card read later never promises a reset that has passed.
function withReset(error: FailureDetails, at: number | undefined): FailureDetails {
    if (error.retryAfterSeconds === undefined || !RESETS.has(error.kind ?? "")) return error;
    const waited = at === undefined ? 0 : Math.max(0, (Date.now() - at) / 1000);
    const left = error.retryAfterSeconds - waited;
    return left > 0 ? { ...error, summary: `${error.summary} ${resetWords(left)}` } : error;
}

export function ErrorCard({
    text,
    at,
    personaId,
    teammate,
    backendId,
    onChooseAccount,
    onRetry,
    onRunAgain,
    onProviders,
}: {
    text: string;
    /** When the notice was written. */
    at?: number;
    personaId?: string;
    teammate?: string;
    backendId?: string;
    onChooseAccount?(): void;
    onRetry?: () => void;
    /** Runs the failed turn again; offered only on the latest one. */
    onRunAgain?: () => Promise<unknown>;
    /** Opens Settings › Providers, the fix for Hotline Agent with no provider. */
    onProviders?: () => void;
}) {
    const error = withReset(errorDetails(text), at);
    const [again, setAgain] = useState<"idle" | "trying" | Refused>("idle");
    const [offer, setOffer] = useState<AccountOffer>("none");
    const account = personaId !== undefined && backendId !== undefined && aboutAccount(error);
    // A turn moved to another account is worth another try whatever stopped it; while the move is offered, the move is the choice.
    const worthIt = offer === "moved" || (error.kind !== undefined && WORTH_ANOTHER_TRY.has(error.kind));
    const offerAgain = onRunAgain !== undefined && offer !== "open" && worthIt;
    return (
        <section className="my-2 min-w-0 max-w-full rounded-lg border border-line bg-raised p-3" aria-label={error.title}>
            {account ? (
                <AccountFailure
                    error={error}
                    personaId={personaId}
                    teammate={teammate}
                    backendId={backendId}
                    latest={onRunAgain !== undefined}
                    onChooseAccount={onChooseAccount}
                    onRetry={onRetry}
                    onOffer={setOffer}
                />
            ) : (
                <>
                    <div className="flex items-center gap-2 text-danger">
                        <WarningIcon className="shrink-0" />
                        <span className="font-medium">{error.title}</span>
                    </div>
                    <p className="mt-1 text-sm text-ink-2">{error.summary}</p>
                </>
            )}
            {error.kind === "no_provider" && onProviders && (
                <div className="mt-2">
                    <button type="button" className="control btn btn-sm" onClick={onProviders}>
                        Settings › Providers
                    </button>
                </div>
            )}
            {!account && error.signIn && personaId && <AgentSignIn personaId={personaId} action={error.signIn} {...(onRetry ? { onRetry } : {})} />}
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
