/**
 * A line of a message as words, for a place that shows it as one plain line
 * (a search result): the markdown the bubble renders is resolved, not shown.
 * A link is its label, a table row its cells, emphasis and code just their
 * text; headings, quotes and list marks go, and a table's rule line is dropped.
 */
export function excerptText(text: string): string {
	return text
		.split("\n")
		.filter((line) => !/^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/.test(line))
		.map((line) => {
			const row = line.trim();
			const cells = row.startsWith("|") ? row.replace(/^\||\|$/g, "").split("|").map((cell) => cell.trim()).filter((cell) => cell !== "") : null;
			return (cells ? cells.join(" · ") : row).replace(/^(?:#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)/, "");
		})
		.join(" ")
		.replace(/!\[[^\]]*\]\([^)]*\)/g, "")
		.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
		.replace(/(\*\*|__)(.+?)\1/g, "$2")
		.replace(/(^|[\s(])[*_]([^*_\s][^*_]*?)[*_](?=[\s).,;:!?]|$)/g, "$1$2")
		.replace(/`([^`]*)`/g, "$1")
		.replace(/\s+/g, " ")
		.trim();
}
