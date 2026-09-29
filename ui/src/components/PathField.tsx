import { FolderIcon } from "../icons";
import { chooseFolder } from "../serverFiles";

/**
 * A path you can type or pick. The picker is the system's folder chooser
 * (or, on a desk on a server, the window's browser of the server's disk),
 * a key inside the field's end; the field stays so a path you already know
 * how to write is still just words.
 */
export function PathField({
	id,
	value,
	placeholder,
	onChange,
	onCommit,
}: {
	id: string;
	value: string;
	placeholder?: string;
	onChange(value: string): void;
	onCommit?(value: string): void;
}) {
	const choose = async () => {
		const dir = await chooseFolder(value);
		if (dir === null) return;
		onChange(dir);
		onCommit?.(dir);
	};
	return (
		<div className="relative">
			<input
				id={id}
				className="field pr-9 font-mono text-sm"
				spellCheck={false}
				placeholder={placeholder}
				value={value}
				onChange={(event) => onChange(event.target.value)}
				onBlur={() => onCommit?.(value)}
			/>
			<button
				type="button"
				className="control btn-icon btn-quiet absolute top-1/2 right-0.5 -translate-y-1/2"
				title="Choose a folder"
				aria-label="Choose a folder"
				onClick={() => void choose()}
			>
				<FolderIcon />
			</button>
		</div>
	);
}
