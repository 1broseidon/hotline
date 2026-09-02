import { FolderIcon } from "../icons";
import { pickDirectory } from "../native";

/**
 * A path you can type or pick. The picker is the system's folder chooser;
 * the field stays so a path you already know how to write is still just words.
 */
export function PathField({
	id,
	value,
	onChange,
	onCommit,
}: {
	id: string;
	value: string;
	onChange(value: string): void;
	onCommit?(value: string): void;
}) {
	const choose = async () => {
		const dir = await pickDirectory();
		if (dir === null) return;
		onChange(dir);
		onCommit?.(dir);
	};
	return (
		<div className="flex items-center gap-2">
			<input
				id={id}
				className="field min-w-0 flex-1 font-mono text-xs"
				spellCheck={false}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				onBlur={() => onCommit?.(value)}
			/>
			<button type="button" className="btn-quiet inline-flex items-center gap-1.5" onClick={() => void choose()}>
				<FolderIcon />
				Choose
			</button>
		</div>
	);
}
