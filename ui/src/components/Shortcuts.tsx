import { type Chord, CHORDS, CHORD_GROUPS, chordKeys, withCurrentKeys } from "../chords";
import { useHotkeys } from "../hotkeys";
import { CloseIcon } from "../icons";
import { isDesktop } from "../native";
import { Band } from "../ui/Band";
import { Scroll } from "../ui/Scroll";
import { useDictationAvailable } from "../voice/dictation";

/**
 * Every chord the window hears, as a pane. The rows are the table the
 * keydown handler matches, so a shortcut Help forgot is not a shortcut.
 * The system-wide ones show the keys picked in Settings › General, and
 * only where this computer can do what they do.
 */
export function Shortcuts({ onClose }: { onClose(): void }) {
	useHotkeys();
	const dictation = useDictationAvailable();
	const offered = (chord: Chord) => chord.hotkey === undefined || (isDesktop() && (chord.hotkey !== "dictate" || dictation));
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
					{CHORD_GROUPS.map((group) => {
						const rows = CHORDS.filter((chord) => chord.group === group.id && offered(chord)).map(withCurrentKeys);
						if (rows.length === 0) return null;
						return (
							<section key={group.id}>
								<h3 className="group-title">{group.title}</h3>
								<div className="grouped">
									{rows.map((chord) => (
										<div key={chord.id} className="group-row">
											<span className="group-row-text">
												<span className="group-row-title">{chord.label}</span>
											</span>
											{chord.keys === "" ? <span className="text-sm text-ink-3">Off</span> : <Keys keys={chord.keys} />}
										</div>
									))}
								</div>
								{group.id === "anywhere" && <p className="group-hint">Pick their keys in Settings › General.</p>}
							</section>
						);
					})}
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
