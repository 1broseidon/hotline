import { useMemo, useState } from "react";
import { highlight } from "../highlight";
import { WrapIcon } from "../icons";
import { CopyKey, Sheet, SheetKey } from "./Sheet";

/** Below this many lines a block is read at a glance, and numbers would be furniture. */
const NUMBERED_FROM = 6;

/**
 * A block of code on its sheet: the language, and the file when the fence
 * names one, on the head with Wrap and Copy. A long line scrolls across under
 * a fade rather than being cut, until Wrap folds it; Copy takes the code as
 * written, never the line numbers.
 */
export function CodeSheet({ code, language, file }: { code: string; language: string | undefined; file: string | undefined }) {
	const [wrap, setWrap] = useState(false);
	const lines = useMemo(() => highlight(code, language), [code, language]);
	return (
		<Sheet
			kind="code"
			label={
				<>
					{language ?? "Code"}
					{file !== undefined && (
						<>
							{" · "}
							<b>{file}</b>
						</>
					)}
				</>
			}
			keys={
				<>
					<SheetKey icon={<WrapIcon />} label="Wrap" title="Wrap long lines" pressed={wrap} onClick={() => setWrap((was) => !was)} />
					<CopyKey text={code} title="Copy the code" />
				</>
			}
		>
			<pre className="code-body" data-wrap={wrap || undefined} data-numbered={lines.length >= NUMBERED_FROM || undefined}>
				<code>
					{lines.map((runs, index) => (
						// biome-ignore lint/suspicious/noArrayIndexKey: a line is its place in the block.
						<span key={index} className="code-line" data-n={index + 1}>
							{runs.map((run, at) =>
								run.kind === undefined ? (
									run.text
								) : (
									// biome-ignore lint/suspicious/noArrayIndexKey: a run is its place in the line.
									<span key={at} className={`code-${run.kind}`}>
										{run.text}
									</span>
								),
							)}
						</span>
					))}
				</code>
			</pre>
		</Sheet>
	);
}

/**
 * The language and file a fence's info string names: ```python setup.py,
 * ```python title="setup.py" or ```python:setup.py.
 */
export function fenceOf(language: string | undefined, meta: string | undefined): { language: string | undefined; file: string | undefined } {
	const [lang, joined] = language?.split(":", 2) ?? [];
	const titled = meta?.match(/title="([^"]+)"/)?.[1];
	const first = meta?.trim().split(/\s+/)[0];
	const named = titled ?? joined ?? (first !== undefined && first !== "" && !/[={]/.test(first) ? first : undefined);
	return { language: lang === "" ? undefined : lang, file: named };
}
