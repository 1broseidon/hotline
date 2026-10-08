import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { matchChord, withCurrentKeys, CHORDS } from "../chords";
import { HOTKEYS, hotkeyFromPress, hotkeyLabel, setHotkey, setRecording, useHotkeyRefusals, useHotkeys, type HotkeyId } from "../hotkeys";
import { CloseIcon } from "../icons";
import { isDesktop, platform } from "../native";
import { useDictationAvailable } from "../voice/dictation";

const HOLD = platform() === "macos" ? "Hold Control, Option or Command, then press a key." : "Hold Ctrl or Alt, then press a key.";

/**
 * Settings › General › Shortcuts: the keys that work from any app on this
 * computer (hotkeys.ts). Each row records new keys, turns its shortcut off,
 * and says when the system would not give Hotline the keys. Dictate is
 * offered only where this Mac can dictate.
 */
export function HotkeysSection() {
	const bindings = useHotkeys();
	const refused = useHotkeyRefusals();
	const dictation = useDictationAvailable();
	if (!isDesktop()) return null;
	const rows = HOTKEYS.filter((hotkey) => hotkey.id !== "dictate" || dictation);
	return (
		<section>
			<h3 className="group-title">Shortcuts</h3>
			<div className="grouped">
				{rows.map((hotkey) => (
					<HotkeyRow
						key={hotkey.id}
						id={hotkey.id}
						label={hotkey.label}
						keys={bindings[hotkey.id]}
						refusal={refused[hotkey.id]}
						usedBy={(accelerator) => rows.find((other) => other.id !== hotkey.id && bindings[other.id] === accelerator)?.label}
					/>
				))}
			</div>
			<p className="group-hint">
				{dictation
					? "They work from any app and bring Hotline forward. Dictate starts or stops dictation in the open conversation; Call calls the open teammate, or hangs up."
					: "It works from any app and brings Hotline forward. Call calls the open teammate, or hangs up."}
			</p>
		</section>
	);
}

function HotkeyRow({
	id,
	label,
	keys,
	refusal,
	usedBy,
}: {
	id: HotkeyId;
	label: string;
	keys: string;
	refusal: string | undefined;
	usedBy(accelerator: string): string | undefined;
}) {
	const [recording, setRecordingHere] = useState(false);
	const [note, setNote] = useState<string | null>(null);
	// A Space that finished recording would otherwise click the key on its way up and start again.
	const swallowKeyUp = useRef(false);

	// The shortcuts let go while keys are recorded, so pressing the ones
	// already bound reaches this field instead of doing their job.
	useEffect(() => {
		if (!recording) return;
		setRecording(true);
		return () => setRecording(false);
	}, [recording]);

	const record = (event: KeyboardEvent<HTMLButtonElement>) => {
		const press = event.nativeEvent;
		// Tab still moves on, and leaving the field stops recording.
		if (press.key === "Tab" && !press.ctrlKey && !press.altKey && !press.metaKey) return;
		event.preventDefault();
		event.stopPropagation();
		const held = press.ctrlKey || press.altKey || press.metaKey || press.shiftKey;
		if (press.key === "Escape" && !held) {
			setRecordingHere(false);
			setNote(null);
			return;
		}
		const accelerator = hotkeyFromPress(press);
		if (accelerator === null) {
			if (!["Control", "Alt", "Shift", "Meta"].includes(press.key)) setNote(HOLD);
			return;
		}
		const other = usedBy(accelerator);
		if (other !== undefined) {
			setNote(`${other} already uses ${hotkeyLabel(accelerator)}.`);
			return;
		}
		const chord = matchChord(press);
		if (chord !== null) {
			const row = CHORDS.find((one) => one.id === chord || (one.id === "teammate-seat" && chord.startsWith("teammate-")));
			setNote(`${withCurrentKeys(row!).keys} is Hotline's own ${row!.label} shortcut.`);
			return;
		}
		setHotkey(id, accelerator);
		setRecordingHere(false);
		setNote(null);
		swallowKeyUp.current = true;
	};

	const message = note ?? refusal;
	return (
		<div className="group-row">
			<span className="group-row-text">
				<span className="group-row-title">{label}</span>
				{message !== undefined && <span className={`group-row-detail whitespace-normal ${note === null ? "text-danger" : ""}`}>{message}</span>}
			</span>
			<span className="flex items-center gap-1">
				<button
					type="button"
					className="control btn btn-sm min-w-24 justify-center"
					aria-label={recording ? `Press the keys for ${label}` : `${label} shortcut: ${keys === "" ? "off" : hotkeyLabel(keys)}. Change`}
					aria-pressed={recording}
					onClick={() => {
						setNote(null);
						setRecordingHere(true);
					}}
					onKeyDown={recording ? record : undefined}
					onKeyUp={(event) => {
						if (!swallowKeyUp.current) return;
						swallowKeyUp.current = false;
						event.preventDefault();
					}}
					onBlur={() => {
						setRecordingHere(false);
						setNote(null);
					}}
				>
					{recording ? "Press keys…" : keys === "" ? "Off" : <kbd className="kbd">{hotkeyLabel(keys)}</kbd>}
				</button>
				{keys !== "" && !recording && (
					<button type="button" className="chip-x" title="Turn off" aria-label={`Turn off the ${label} shortcut`} onClick={() => setHotkey(id, "")}>
						<CloseIcon />
					</button>
				)}
			</span>
		</div>
	);
}
