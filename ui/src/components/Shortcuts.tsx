import { CHORDS, CHORD_GROUPS, chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { Band } from "../ui/Band";
import { Scroll } from "../ui/Scroll";

/**
 * Every chord the window hears, as a pane. The rows are the table the
 * keydown handler matches, so a shortcut Help forgot is not a shortcut.
 */
export function Shortcuts({ onClose }: { onClose(): void }) {
	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Keyboard shortcuts</h2>
				<button
					type="button"
					className="control btn-icon"
					title={`Close (${chordKeys("close")})`}
					aria-label="Close"
					onClick={onClose}
				>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{CHORD_GROUPS.map((group) => (
						<section key={group.id}>
							<h3 className="group-title">{group.title}</h3>
							<div className="grouped">
								{CHORDS.filter((chord) => chord.group === group.id).map((chord) => (
									<div key={chord.id} className="group-row">
										<span className="group-row-text">
											<span className="group-row-title">{chord.label}</span>
										</span>
										<Keys keys={chord.keys} />
									</div>
								))}
							</div>
						</section>
					))}
				</div>
			</Scroll>
		</div>
	);
}

function Keys({ keys }: { keys: string }) {
	const parts = keys.split("+");
	return (
		<span className="flex items-center gap-0.5" aria-label={keys}>
			{parts.map((part, index) => (
				<span key={`${part}-${index}`} className="flex items-center gap-0.5">
					{index > 0 && <span className="text-ink-4">+</span>}
					<kbd className="kbd">{part}</kbd>
				</span>
			))}
		</span>
	);
}
