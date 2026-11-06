import { createContext, type ReactNode, useContext } from "react";
import { chordKeys } from "../chords";
import { ArrowLeftIcon } from "../icons";
import { Avatar } from "./Avatar";
import { Band } from "./Band";

/** Who said a message and when: the bubble's, for an open view's band. */
export type Said = { personaId: string; ts: number };

/** The bubble a thing sits in: who said it and when. None outside a teammate's bubble. */
export const SaidBy = createContext<Said | null>(null);

/** A teammate as an open view's band shows them. */
export type Speaker = { id: string; name: string; avatar?: { hash?: string | undefined } | undefined };

/** How the window finds a teammate by id, for the band's face and name. The window provides it. */
export const Speakers = createContext<((personaId: string) => Speaker | undefined) | null>(null);

/** When something was said, as the band says it: just now, 4 min ago, 3 h ago, yesterday, then the date. */
export function agoOf(at: number, now: number = Date.now()): string {
	const minutes = Math.round((now - at) / 60_000);
	if (minutes < 1) return "just now";
	if (minutes < 60) return `${minutes} min ago`;
	const hours = Math.round(minutes / 60);
	if (hours < 24) return `${hours} h ago`;
	if (hours < 48) return "yesterday";
	return new Date(at).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/**
 * The band over anything opened from a conversation: a table, code or a
 * diagram in the conversation's place, a picture, a capture or a file over
 * the window. One look for all of them (docs/design/open-view.html): ← back
 * at the left (Escape too), the teammate's face and name and when they said
 * it, then at the right what it is as a fact in the instrument face and its
 * keys, each with its word. No title, no Close, no line under it.
 */
export function OpenBand({
	onBack,
	from,
	fact,
	children,
	backRef,
}: {
	onBack(): void;
	/** Who said it and when; the band's own SaidBy when not given. */
	from?: Said | null | undefined;
	fact: string;
	children?: ReactNode;
	backRef?: React.Ref<HTMLButtonElement> | undefined;
}) {
	const said = useContext(SaidBy);
	const speakers = useContext(Speakers);
	const by = from === undefined ? said : from;
	const speaker = by !== null ? speakers?.(by.personaId) : undefined;
	return (
		<div className="open-band">
			<Band>
				<button ref={backRef} type="button" className="control btn-icon -ml-1" title={`Back (${chordKeys("close")})`} aria-label="Back to the conversation" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				{speaker !== undefined && (
					<>
						<Avatar id={speaker.id} name={speaker.name} size={20} hash={speaker.avatar?.hash} />
						<span className="open-band-name">{speaker.name}</span>
					</>
				)}
				{by !== null && <span className="instrument">· {agoOf(by.ts)}</span>}
				<span className="min-w-0 flex-1" />
				<span className="instrument open-band-fact">{fact}</span>
				{children}
			</Band>
		</div>
	);
}

/** The speaker's name for a caption, from the band's same sources. */
export function useSpeaker(from: Said | null | undefined): Speaker | undefined {
	const speakers = useContext(Speakers);
	return from != null ? speakers?.(from.personaId) : undefined;
}
