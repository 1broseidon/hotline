import { useEffect, useState } from "react";
import type { ConfigChoice } from "../generated/contract";
import { accountName, plainModel } from "../accounts";
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

export function errorDetails(text: string): { title: string; summary: string; details: string; context?: string; signIn?: AgentSignInAction; kind?: string; model?: string } {
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
                };
            }
        } catch { /* Older notices are plain text. */ }
    }
    let details = text;
    let reported: unknown;
    const json = text.indexOf("{");
    if (json !== -1) {
        try {
            reported = JSON.parse(text.slice(json));
            details = text.slice(0, json) + JSON.stringify(reported, null, 2);
        } catch { /* Keep the original diagnostic. */ }
    }
    return { ...reportedWords(json === -1 ? text : text.slice(0, json), reported), details };
}

/** How long a report's head can be and still read as one: "Claude Code stopped", not a sentence cut at a colon. */
const HEAD_MOST = 60;

/**
 * A plain report in its own words: its first line, split at a colon into
 * what happened and why when the part before it is short enough to be a
 * head ("Claude Code stopped: The usage limit resets at 9 PM."). A report
 * that is only JSON says it with its first `message`.
 */
function reportedWords(prose: string, reported: unknown): { title: string; summary: string } {
    const line = prose.split("\n").map((one) => one.trim()).find((one) => one !== "")?.replace(/[\s:]+$/, "") ?? "";
    const said = line !== "" ? line : (messageIn(reported) ?? "");
    const colon = said.indexOf(": ");
    if (colon > 0 && colon <= HEAD_MOST) return { title: said.slice(0, colon), summary: said.slice(colon + 2) };
    if (said !== "" && said !== "Turn failed") return { title: "Turn failed", summary: said.length > 300 ? `${said.slice(0, 300)}…` : said };
    return { title: "Turn failed", summary: "The whole report is under Details." };
}

/** The first `message` a reported JSON error carries, however deep. */
function messageIn(value: unknown): string | undefined {
    if (value === null || typeof value !== "object") return undefined;
    const record = value as Record<string, unknown>;
    if (typeof record["message"] === "string" && record["message"].trim() !== "") return record["message"].trim();
    for (const inner of Object.values(record)) {
        const found = messageIn(inner);
        if (found !== undefined) return found;
    }
    return undefined;
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
        <section className="error-card" aria-label={error.title}>
            <div className="error-card-head">
                <WarningIcon className="shrink-0" />
                <span className="selectable">{error.title}</span>
            </div>
            <p className="error-card-said selectable">{error.summary}</p>
            {error.kind === "no_provider" && onProviders && (
                <div className="mt-2">
                    <button type="button" className="control btn btn-sm" onClick={onProviders}>
                        Settings › Providers
                    </button>
                </div>
            )}
            {error.kind === "account_gone" && error.model !== undefined && personaId && (
                <OtherAccounts personaId={personaId} model={error.model} {...(onRunAgain ? { onRunAgain } : {})} />
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
            <details className="error-card-details">
                <summary>Details</summary>
                {error.context && <p className="mt-2 break-words text-xs text-ink-3" style={{ overflowWrap: "anywhere" }}>{error.context}</p>}
                <pre className="selectable mt-2 max-h-80 max-w-full overflow-auto whitespace-pre-wrap break-words text-xs text-ink-2" style={{ overflowWrap: "anywhere" }}>{error.details}</pre>
            </details>
        </section>
    );
}

/**
 * A teammate whose account is gone: the other accounts that serve the same
 * model, one button each. Picking one moves the teammate there and runs the
 * failed turn again; nothing moves it without a press.
 */
function OtherAccounts({ personaId, model, onRunAgain }: { personaId: string; model: string; onRunAgain?: () => Promise<unknown> }) {
    const [others, setOthers] = useState<ConfigChoice[] | null>(null);
    const [state, setState] = useState<"idle" | "moving" | Refused>("idle");
    useEffect(() => {
        let live = true;
        void wire.command("models.list", {}).then(
            (choices) => live && setOthers(choices.filter((choice) => plainModel(choice.id) === model)),
            () => live && setOthers([]),
        );
        return () => {
            live = false;
        };
    }, [model]);
    if (others === null) return null;
    if (others.length === 0) return <p className="mt-2 text-sm text-ink-3">No other account serves this model. Add one in Settings › Providers.</p>;
    return (
        <div className="mt-2 flex flex-wrap items-center gap-2">
            {others.map((choice) => (
                <button
                    key={choice.id}
                    type="button"
                    className="control btn btn-sm"
                    disabled={state === "moving"}
                    onClick={() => {
                        setState("moving");
                        void wire
                            .command("session.set_model", { personaId, modelId: choice.id })
                            .then(() => onRunAgain?.())
                            .then(() => setState("idle"))
                            .catch((reason: unknown) => setState(refused("Couldn't move it", reason)));
                    }}
                >
                    Use {choice.account ? accountName(choice.group ?? "", choice.account.label, choice.account.shared) : (choice.group ?? choice.name)}
                </button>
            ))}
            {typeof state === "object" && <Refusal {...state} />}
        </div>
    );
}
