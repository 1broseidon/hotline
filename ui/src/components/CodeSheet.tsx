import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { highlight } from "../highlight";
import { Sheet } from "./Sheet";

/** More lines than this and code is cut in a bubble, however narrow, as on the phone. */
const INLINE_LINES = 12;

/** Below this many lines a block is read at a glance, and numbers would be furniture. */
const NUMBERED_FROM = 6;

/**
 * A block of code on its sheet: the language, and the file when the fence
 * names one, on the head. Code that fits the bubble reads in it whole. Code
 * wider than the bubble, or longer than a glance, is cut: its first lines,
 * faded where they go on, how many there are, and a click for all of it.
 * Copy takes the code as written, never the line numbers.
 */
export function CodeSheet({ code, language, file }: { code: string; language: string | undefined; file: string | undefined }) {
	const lines = useMemo(() => highlight(code, language), [code, language]);
	const body = useRef<HTMLPreElement>(null);
	const [wide, setWide] = useState(false);
	useLayoutEffect(() => {
		const box = body.current;
		if (box === null) return;
		const look = () => setWide(box.scrollWidth > box.clientWidth + 2);
		look();
		const resized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(look);
		// The box for the bubble's width, its lines for code streaming in.
		resized?.observe(box);
		if (box.firstElementChild !== null) resized?.observe(box.firstElementChild);
		return () => resized?.disconnect();
	}, []);
	const tall = lines.length > INLINE_LINES;
	const title = [language ?? "Code", file].filter(Boolean).join(" · ");
	return (
		<Sheet
			kind="code"
			copy={{ text: code, title: "Copy code" }}
			open={{ kind: "code", text: fence(code, language, file), title }}
			cut={wide || tall ? { summary: `${lines.length} ${lines.length === 1 ? "line" : "lines"}`, wide, tall } : null}
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
			keys={null}
		>
			<pre ref={body} className="code-body" data-numbered={lines.length >= NUMBERED_FROM || undefined}>
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

/** The block as a fence again, so the full view draws it the same way. */
function fence(code: string, language: string | undefined, file: string | undefined): string {
	const info = [language ?? "", file !== undefined ? `title="${file}"` : ""].join(" ").trim();
	const ticks = "`".repeat(Math.max(3, ...[...code.matchAll(/`+/g)].map((run) => run[0].length + 1)));
	return `${ticks}${info}\n${code}\n${ticks}`;
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
