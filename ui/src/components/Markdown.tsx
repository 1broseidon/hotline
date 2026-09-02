import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openLink } from "../native";

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
 * touches `innerHTML`, and `rehype-raw` is deliberately absent.
 *
 * Only an agent's bubbles come through here. What you typed is shown as you
 * typed it.
 */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
	return (
		<div className="md">
			<ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS} skipHtml>
				{text}
			</ReactMarkdown>
		</div>
	);
});

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

	table: ({ children }) => (
		<div className="overflow-x-auto">
			<table>{children}</table>
		</div>
	),
};
