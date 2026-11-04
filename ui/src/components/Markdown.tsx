import { Children, cloneElement, createContext, isValidElement, memo, type ReactElement, type ReactNode, useContext } from "react";
import ReactMarkdown, { type Components, type ExtraProps } from "react-markdown";
import remarkGfm from "remark-gfm";
import { WarningIcon } from "../icons";
import { openLink } from "../native";
import { CodeSheet, fenceOf } from "./CodeSheet";
import { Favicon } from "./Favicon";
import { Mermaid } from "./Mermaid";
import { CopyKey, OpenInPane, OpenKey, Sheet } from "./Sheet";

/** The syntax tree's element react-markdown hands a component, and what can stand inside one. */
type Element = NonNullable<ExtraProps["node"]>;
type ElementContent = Element["children"][number];

/**
 * The markdown an agent is allowed to speak inside a bubble.
 *
 * A messenger has no document structure to offer, so this is deliberately not
 * a full renderer. Emphasis, code, links, lists, quotes and tables survive
 * because a person types those into a chat; every level of heading arrives as
 * one, a step up from the words, because a person would not send you an
 * `<h2>`; images and raw HTML do not survive at all. A table, a block of code
 * and a diagram are objects the message carries, each on one sheet
 * (Sheet.tsx); a table and a diagram open at full width in the pane.
 *
 * Everything here comes out of a language model, so what matters is that no
 * HTML is ever interpreted: `react-markdown` builds React nodes and never
 * touches `innerHTML`, and `rehype-raw` is deliberately absent. A ```mermaid
 * block is drawn, as an image that cannot run anything (see Mermaid.tsx).
 *
 * Only an agent's bubbles come through here, and a release's notes in
 * Settings › Updates. What you typed is shown as you typed it. A markdown
 * file a teammate sent is a document, and opens as one (`MarkdownDocument`).
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
		<MarkdownSource.Provider value={text}>
			<div className="md">{ReactMarkdown({ remarkPlugins: [remarkGfm], components: COMPONENTS, skipHtml: true, children: text })}</div>
		</MarkdownSource.Provider>
	);
	trees.set(text, tree);
	if (trees.size > KEPT) trees.delete(trees.keys().next().value as string);
	return tree;
}

/**
 * A markdown file opened from its card: a document, so its headings and rules
 * stand as they were written. The rest is a bubble's: no HTML, no remote
 * images, links open in the browser, tables and code on their sheets.
 */
export function MarkdownDocument({ text }: { text: string }) {
	return (
		<MarkdownSource.Provider value={text}>
			{/* A document is already open at full size, over the window. */}
			<OpenInPane.Provider value={null}>
				<article className="md md-doc">
					<ReactMarkdown remarkPlugins={[remarkGfm]} components={DOCUMENT} skipHtml>
						{text}
					</ReactMarkdown>
				</article>
			</OpenInPane.Provider>
		</MarkdownSource.Provider>
	);
}

/** The markdown being drawn, so an object can copy or open exactly what was written for it. */
const MarkdownSource = createContext("");

/**
 * Inside a table, how each column reads, by its place: a column of numbers
 * (`num`) or of sentences (`wide`); null outside a table.
 */
type ColumnKind = "num" | "wide";
const TableColumns = createContext<Map<number, ColumnKind> | null>(null);

const COMPONENTS: Components = {
	/* Six sizes of heading in a chat bubble is a document pretending to be a
	 * message; every level lands as one heading, a step up from the words. */
	h1: ({ children }) => <p className="md-heading">{children}</p>,
	h2: ({ children }) => <p className="md-heading">{children}</p>,
	h3: ({ children }) => <p className="md-heading">{children}</p>,
	h4: ({ children }) => <p className="md-heading">{children}</p>,
	h5: ({ children }) => <p className="md-heading">{children}</p>,
	h6: ({ children }) => <p className="md-heading">{children}</p>,

	/* A rule inside a bubble has nothing to divide. */
	hr: () => null,

	/* Remote images would let a message's author learn when it was read, and
	 * an agent that wants to show you a picture can describe it instead. */
	img: ({ alt }) => (alt ? <span className="text-ink-3">{alt}</span> : null),

	/* The desk opens the URL in the system browser. A bubble does not get to
	 * spawn a window of its own. In a table a link is a source, so it is a
	 * site's chip: its icon when the desk has one, and its words. */
	a: function Link({ children, href }) {
		const inTable = useContext(TableColumns) !== null;
		const site = inTable ? siteOf(href) : null;
		return (
			<a
				href={href}
				className={site === null ? undefined : "site-chip"}
				onClick={(event) => {
					event.preventDefault();
					if (href) void openLink(href);
				}}
			>
				{site !== null && <Favicon site={site} size={14} />}
				{site !== null && children === href ? site : children}
			</a>
		);
	},

	/* A fenced block tagged mermaid is a diagram; every other block is code. */
	pre: ({ children, node }) => {
		const code = codeOf(children, node);
		if (code === null) return <pre>{children}</pre>;
		if (code.language === "mermaid") return <Mermaid source={code.text} />;
		return <CodeSheet code={code.text} {...fenceOf(code.language, code.meta)} />;
	},

	/* A list's number counts from where the markdown said it starts. */
	ol: ({ children, start }) => <ol style={start !== undefined && start !== 1 ? { counterReset: `md-item ${start - 1}` } : undefined}>{children}</ol>,

	blockquote: ({ children, node }) => {
		const alert = alertOf(node);
		if (alert === null) return <blockquote>{children}</blockquote>;
		const warns = alert === "warning" || alert === "caution";
		return (
			<div className="callout" data-alert={alert} role="note">
				{warns && <WarningIcon className="callout-icon" />}
				<div className="callout-words">
					<span className="callout-label">{ALERT_LABELS[alert]}</span>
					{withoutMarker(children)}
				</div>
			</div>
		);
	},

	table: function Table({ children, node }) {
		const source = useContext(MarkdownSource);
		const markdown = sourceOf(source, node);
		const { rows, columns, kinds } = shapeOf(node);
		return (
			<Sheet
				kind="table"
				label={
					<>
						Table · <b>{counted(rows, "row")}</b> · {counted(columns, "column")}
					</>
				}
				keys={
					<>
						<OpenKey object={{ kind: "table", text: markdown }} />
						<CopyKey text={markdown} title="Copy as Markdown" />
					</>
				}
			>
				<div className="md-table">
					<TableColumns.Provider value={kinds}>
						<table>{children}</table>
					</TableColumns.Provider>
				</div>
			</Sheet>
		);
	},

	/* A column of numbers stands right, so its digits line up; a column of
	 * sentences keeps a reading width rather than folding to a word a line. */
	tr: function Row({ children }) {
		const kinds = useContext(TableColumns);
		let column = 0;
		return (
			<tr>
				{Children.map(children, (cell) => {
					if (!isValidElement<{ className?: string }>(cell)) return cell;
					const kind = kinds?.get(column++);
					return kind === undefined ? cell : cloneElement(cell, { className: kind });
				})}
			</tr>
		);
	},
};

const counted = (count: number, noun: string) => `${count} ${noun}${count === 1 ? "" : "s"}`;

/** A link's site, for its chip: only a web page's has one. */
function siteOf(href: string | undefined): string | null {
	if (href === undefined || !/^https?:/i.test(href)) return null;
	try {
		return new URL(href).hostname.replace(/^www\./, "");
	} catch {
		return null;
	}
}

/** The code a fenced or indented block holds, from the `<code>` react-markdown puts inside its `<pre>`. */
function codeOf(children: ReactNode, node: Element | undefined): { text: string; language: string | undefined; meta: string | undefined } | null {
	if (!isValidElement<{ className?: string; children?: ReactNode }>(children)) return null;
	const { className, children: text } = children.props;
	if (typeof text !== "string") return null;
	const language = className
		?.split(" ")
		.find((name) => name.startsWith("language-"))
		?.slice("language-".length);
	const inner = node?.children.find((child): child is Element => child.type === "element" && child.tagName === "code");
	const meta = (inner?.data as { meta?: unknown } | undefined)?.meta;
	return { text: text.replace(/\n$/, ""), language, meta: typeof meta === "string" ? meta : undefined };
}

/**
 * The markdown a node was written as, cut from the message. A table inside a
 * list or a quote has its lines prefixed by that, so every line after the
 * first gives up as many characters as the first one started in.
 */
function sourceOf(source: string, node: Element | undefined): string {
	const start = node?.position?.start;
	const end = node?.position?.end;
	if (start?.offset === undefined || end?.offset === undefined) return "";
	const indent = start.column - 1;
	return source
		.slice(start.offset, end.offset)
		.split("\n")
		.map((line, index) => (index === 0 ? line : line.slice(indent)))
		.join("\n");
}

/** A cell that is a number, a money sum, a share or a measured range: `5–8 ms`, `$80`, `42%`, `~1.2k`. */
const NUMERIC = /^[~≈<>≤≥±+\-−]?\s*[$€£¥]?\d[\d,.]*\s*(?:[–-]\s*[$€£¥]?\d[\d,.]*)?\s*(?:%|[A-Za-zµ]{1,3}(?:\/[A-Za-z]{1,3})?)?$/;

/** Past this many characters on average, a column holds sentences. */
const SENTENCES = 40;

/** How many rows and columns a table has, and which of its columns hold only numbers, or sentences. */
function shapeOf(node: Element | undefined): { rows: number; columns: number; kinds: Map<number, ColumnKind> } {
	const part = (tag: string) => node?.children.find((child): child is Element => child.type === "element" && child.tagName === tag);
	const rowsOf = (section: Element | undefined) => elements(section?.children ?? [], "tr");
	const head = rowsOf(part("thead"))[0];
	const body = rowsOf(part("tbody")).map((row) => elements(row.children, "td").map(textOf));
	const columns = head === undefined ? 0 : elements(head.children, "th").length;
	const kinds = new Map<number, ColumnKind>();
	for (let column = 0; column < columns; column++) {
		const cells = body.map((row) => row[column]?.trim() ?? "").filter((cell) => cell !== "");
		if (cells.length === 0) continue;
		if (cells.every((cell) => NUMERIC.test(cell))) kinds.set(column, "num");
		else if (cells.reduce((sum, cell) => sum + cell.length, 0) / cells.length > SENTENCES) kinds.set(column, "wide");
	}
	return { rows: body.length, columns, kinds };
}

const elements = (nodes: ElementContent[], tag: string): Element[] =>
	nodes.filter((child): child is Element => child.type === "element" && child.tagName === tag);

function textOf(node: ElementContent): string {
	if (node.type === "text") return node.value;
	if (node.type === "element") return node.children.map(textOf).join("");
	return "";
}

/**
 * GitHub's alerts: a quote whose first line is `[!WARNING]` and the like.
 * remark-gfm leaves the marker as text, so it is read here.
 */
type Alert = "note" | "tip" | "important" | "warning" | "caution";
const ALERT_LABELS: Record<Alert, string> = { note: "Note", tip: "Tip", important: "Important", warning: "Warning", caution: "Caution" };
const ALERT_MARKER = /^\s*\[!(note|tip|important|warning|caution)\][ \t]*\n?/i;

function alertOf(node: Element | undefined): Alert | null {
	const first = node?.children.find((child): child is Element => child.type === "element");
	const text = first?.tagName === "p" ? first.children[0] : undefined;
	if (text?.type !== "text") return null;
	return (text.value.match(ALERT_MARKER)?.[1]?.toLowerCase() as Alert | undefined) ?? null;
}

/** The quote's words without the alert's marker, and without the paragraph it stood in when it stood alone. */
function withoutMarker(children: ReactNode): ReactNode {
	let done = false;
	return Children.map(children, (child) => {
		if (done || !isValidElement<{ children?: ReactNode }>(child)) return child;
		done = true;
		const [first, ...rest] = Children.toArray(child.props.children);
		const stripped = typeof first === "string" ? first.replace(ALERT_MARKER, "") : first;
		const kept = stripped === "" ? rest : [stripped, ...rest];
		return kept.length === 0 ? null : cloneElement(child, undefined, ...kept);
	});
}

const { a, img, pre, table, tr, blockquote, ol } = COMPONENTS;
const DOCUMENT: Components = { a, img, pre, table, tr, blockquote, ol } as Components;
