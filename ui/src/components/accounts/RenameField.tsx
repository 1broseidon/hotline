import { useEffect, useRef, useState } from "react";
import { Refusal } from "../../ui/Refusal";

/**
 * A name edited in place. It stays open with what was typed until `onSave`
 * resolves, and shows a refusal under itself, so a name is never dropped.
 */
export function RenameField({
	name,
	busy,
	onSave,
	onCancel,
}: {
	name: string;
	busy: boolean;
	onSave(next: string): Promise<void>;
	onCancel(): void;
}) {
	const [value, setValue] = useState(name);
	const [saving, setSaving] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const input = useRef<HTMLInputElement>(null);
	useEffect(() => input.current?.focus(), []);

	const save = async () => {
		if (saving || busy) return;
		const next = value.trim();
		if (next === "" || next === name) {
			onCancel();
			return;
		}
		setSaving(true);
		setRefusal(null);
		try {
			await onSave(next);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setSaving(false);
		}
	};

	return (
		<span className="flex min-w-0 flex-1 flex-col gap-2">
			<span className="flex min-w-0 gap-2">
				<input
					ref={input}
					className="field min-w-0 flex-1"
					aria-label="Account name"
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={(event) => {
						if (event.key === "Enter") void save();
						if (event.key === "Escape") {
							// Escape here undoes the rename; it does not also leave Settings.
							event.stopPropagation();
							onCancel();
						}
					}}
				/>
				<button type="button" className="control btn btn-primary" disabled={saving || busy} onClick={() => void save()}>
					Save
				</button>
			</span>
			{refusal !== null && <Refusal message={refusal} />}
		</span>
	);
}
