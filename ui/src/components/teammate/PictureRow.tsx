import { useEffect, useState } from "react";
import type { AvatarCandidate, Persona } from "../../generated/contract";
import { faceOf } from "../../ui/Avatar";
import { wire } from "../../wire";
import { FoldRow, NESTED } from "./rows";

/**
 * The teammate's picture, chosen from ones the desk draws on this machine
 * (DiceBear's styles, no image provider needed). Opening the row shows one
 * per style; a press keeps that picture as the person's, so the teammate
 * leaves it be; More draws another set. Each is drawn on the teammate's own
 * colour, as the kept picture is shown, and from the desk's SVG as an image,
 * never as markup in the page.
 */
export function PictureRow({ persona, disabled, onRefusal }: { persona: Persona; disabled: boolean; onRefusal(text: string | null): void }) {
	const [open, setOpen] = useState(false);
	const [round, setRound] = useState(0);
	const [candidates, setCandidates] = useState<AvatarCandidate[] | null>(null);
	const [picking, setPicking] = useState(false);

	useEffect(() => {
		if (!open) return;
		let gone = false;
		setCandidates(null);
		wire.command("avatar.candidates", { personaId: persona.id, round }).then(
			(drawn) => {
				if (!gone) setCandidates(drawn);
			},
			(error: Error) => {
				if (!gone) onRefusal(error.message);
			},
		);
		return () => {
			gone = true;
		};
	}, [open, round, persona.id, onRefusal]);

	const pick = async (candidate: AvatarCandidate) => {
		setPicking(true);
		onRefusal(null);
		try {
			await wire.command("avatar.pick", { personaId: persona.id, style: candidate.style, seed: candidate.seed });
		} catch (error) {
			onRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setPicking(false);
		}
	};

	const value = persona.avatar === undefined ? "Its initial" : persona.avatar.by === "person" ? "Chosen by you" : "Its own";
	return (
		<>
			<FoldRow title="Picture" value={value} open={open} onToggle={() => setOpen((was) => !was)} />
			{open && (
				<div className={NESTED}>
					<div className="picture-choices" aria-busy={candidates === null || picking}>
						{(candidates ?? []).map((candidate) => (
							<button
								key={`${candidate.style}:${candidate.seed}`}
								type="button"
								className="picture-choice"
								title={styleName(candidate.style)}
								aria-label={`Use the ${styleName(candidate.style)} picture`}
								disabled={disabled || picking}
								onClick={() => void pick(candidate)}
							>
								<span className="avatar" style={{ width: 40, height: 40, background: faceOf(persona.id) }}>
									<img className="avatar-picture" src={`data:image/svg+xml,${encodeURIComponent(candidate.svg)}`} alt="" draggable={false} />
								</span>
							</button>
						))}
					</div>
					<button type="button" className="control btn-quiet btn-sm self-start" disabled={disabled || picking || candidates === null} onClick={() => setRound((was) => was + 1)}>
						More
					</button>
				</div>
			)}
		</>
	);
}

/** A DiceBear style's id as its name: "open-peeps" is Open Peeps. */
function styleName(style: string): string {
	return style
		.split("-")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(" ");
}
