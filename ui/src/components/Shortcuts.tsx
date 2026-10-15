import { type Chord, CHORDS, CHORD_GROUPS, withCurrentKeys } from "../chords";
import { useHotkeys } from "../hotkeys";
import { isDesktop } from "../native";
import { useDictationAvailable } from "../voice/dictation";

/**
 * Every chord the window hears, as a section of Settings. The rows are the table the
 * keydown handler matches, so a shortcut Help forgot is not a shortcut.
 * The system-wide ones show the keys picked in Settings › General, and
 * only where this computer can do what they do.
 */
export function ShortcutsSection() {
	useHotkeys();
	const dictation = useDictationAvailable();
	const offered = (chord: Chord) => chord.hotkey === undefined || (isDesktop() && (chord.hotkey !== "dictate" || dictation));
	return (
		<>
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
		</>
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
