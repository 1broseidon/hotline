import type { CredentialKind, LoginPrompt } from "./generated/contract";
import type { Refused } from "./ui/Refusal";

/** Where a flow rests when nothing is in flight: the method list, or one method's field. */
export type Resting = { k: "choosing" } | { k: "entering"; kind: Exclude<CredentialKind, "oauth"> };

/**
 * Connecting one provider, as one state. `preparing` asks the core for a
 * sign-in, `waiting` has the browser page open and polls, `finishing` has a
 * credential (or a key being saved) and is reading models. `refused` names
 * the resting state it fell back to, so every exit shows the reason and a
 * way to try again.
 */
export type SignInFlow =
	| Resting
	| { k: "preparing" }
	| { k: "waiting"; login: LoginPrompt }
	| { k: "finishing"; form: Resting }
	| { k: "refused"; refusal: Refused; back: Resting };

export type SignInEvent =
	| { type: "begin" }
	| { type: "enter"; kind: Exclude<CredentialKind, "oauth"> }
	| { type: "prompted"; login: LoginPrompt }
	| { type: "saving"; form: Resting }
	| { type: "refused"; refusal: Refused; back: Resting }
	| { type: "reset" };

/** The field or list a flow shows when it is not waiting on a browser. */
export function formOf(flow: SignInFlow): Resting {
	return flow.k === "entering" ? flow : flow.k === "refused" ? flow.back : flow.k === "finishing" ? flow.form : { k: "choosing" };
}

/** In flight: the pane is busy and offers only Cancel. */
export function signInBusy(flow: SignInFlow): boolean {
	return flow.k === "preparing" || flow.k === "waiting" || flow.k === "finishing";
}

export function signInStep(flow: SignInFlow, event: SignInEvent): SignInFlow {
	switch (event.type) {
		case "begin":
			return signInBusy(flow) ? flow : { k: "preparing" };
		case "enter":
			return signInBusy(flow) ? flow : { k: "entering", kind: event.kind };
		case "saving":
			return signInBusy(flow) ? flow : { k: "finishing", form: event.form };
		case "prompted":
			// A prompt for a sign-in nobody is waiting on is dropped.
			return flow.k === "preparing" ? { k: "waiting", login: event.login } : flow;
		case "refused":
			return { k: "refused", refusal: event.refusal, back: event.back };
		case "reset":
			return { k: "choosing" };
	}
}

/** The resting state a refusal falls back to: the field a key went in, else the method list. */
export function restingAfter(initial: CredentialKind | null): Resting {
	return initial === null || initial === "oauth" ? { k: "choosing" } : { k: "entering", kind: initial };
}
