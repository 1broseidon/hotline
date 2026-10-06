import { isValidElement, memo, type ReactElement, type ReactNode, useRef, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { CheckIcon, CopyIcon } from "../icons";
import { openLink, writeClipboard } from "../native";
import { Mermaid } from "./Mermaid";

/**
 * The markdown an agent is allowed to speak inside a bubble.
 *
 * A messenger has no document structure to offer, so this is deliberately not
 * a full renderer. Emphasis, code, links, lists and tables survive because a
 * person types those into a chat; headings arrive as bold text because a
 * person would not send you an `<h2>`; images and raw HTML do not survive at
 * all.
 *
 * Everything here comes out of a language model, so what matters is that no
 * HTML is ever interpreted: `react-markdown` builds React nodes and never
 * touches `innerHTML`, and `rehype-raw` is deliberately absent. A ```mermaid
 * block is drawn, as an image that cannot run anything (see Mermaid.tsx).
 *
 * Only an agent's bubbles come through here. What you typed is shown as you
 * typed it.
 */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
	return rendered(text);
});

/** How many messages' parsed trees are kept; a few hundred rows is a teammate's recent tape. */
const KEPT = 500;
const trees = new Map<string, ReactElement>();

/**
 * The tree for a message, parsed once. Switching teammates mounts every row
 * again, and parsing a couple of hundred messages is the whole of the wait; a
 * React element is immutable, so the same one can be drawn again. react-markdown
 * is a plain function of its props, which is what lets it be called here and
 * its answer kept. Least recently drawn goes first.
 */
function rendered(text: string): ReactElement {
	const known = trees.get(text);
	if (known !== undefined) {
		trees.delete(text);
		trees.set(text, known);
		return known;
	}
	const tree = (
		<div className="md">
			{ReactMarkdown({ remarkPlugins: [remarkGfm], components: COMPONENTS, skipHtml: true, children: text })}
		</div>
	);
	trees.set(text, tree);
	if (trees.size > KEPT) trees.delete(trees.keys().next().value as string);
	return tree;
}

const COMPONENTS: Components = {
	/* Six sizes of heading in a chat bubble is a document pretending to be a
	 * message; every level lands as bold body text instead. */
	h1: ({ children }) => <p className="font-semibold">{children}</p>,
	h2: ({ children }) => <p className="font-semibold">{children}</p>,
	h3: ({ children }) => <p className="font-semibold">{children}</p>,
	h4: ({ children }) => <p className="font-semibold">{children}</p>,
	h5: ({ children }) => <p className="font-semibold">{children}</p>,
	h6: ({ children }) => <p className="font-semibold">{children}</p>,

	/* A rule inside a bubble has nothing to divide. */
	hr: () => null,

	/* Remote images would let a message's author learn when it was read, and
	 * an agent that wants to show you a picture can describe it instead. */
	img: ({ alt }) => (alt ? <span className="text-ink-3">{alt}</span> : null),

	/* The desk opens the URL in the system browser. A bubble does not get to
	 * spawn a window of its own. */
	a: ({ children, href }) => (
		<a
			href={href}
			onClick={(event) => {
				event.preventDefault();
				if (href) void openLink(href);
			}}
		>
			{children}
		</a>
	),

	/* A fenced block tagged mermaid is a diagram; every other block is code. */
	pre: ({ children }) => {
		const source = mermaidSource(children);
		return source === null ? <CodeBlock>{children}</CodeBlock> : <Mermaid source={source} />;
	},

	table: ({ children }) => (
		<div className="overflow-x-auto">
			<table>{children}</table>
		</div>
	),
};

/** The source of a ```mermaid block, from the `<code>` react-markdown puts inside its `<pre>`. */
function mermaidSource(children: ReactNode): string | null {
	if (!isValidElement<{ className?: string; children?: ReactNode }>(children)) return null;
	const { className, children: text } = children.props;
	if (!className?.split(" ").includes("language-mermaid")) return null;
	return typeof text === "string" ? text.replace(/\n$/, "") : null;
}

/** A block of code with its copy key in the corner, as on the phone. */
function CodeBlock({ children }: { children: ReactNode }) {
	const pre = useRef<HTMLPreElement>(null);
	const [copied, setCopied] = useState(false);
	return (
		<div className="code-block">
			<pre ref={pre}>{children}</pre>
			<button
				type="button"
				className="code-copy"
				title={copied ? "Copied" : "Copy code"}
				aria-label={copied ? "Copied" : "Copy code"}
				onClick={() => {
					void writeClipboard((pre.current?.textContent ?? "").replace(/\n$/, "")).then(() => {
						setCopied(true);
						setTimeout(() => setCopied(false), 1500);
					});
				}}
			>
				{copied ? <CheckIcon /> : <CopyIcon />}
			</button>
		</div>
	);
}
