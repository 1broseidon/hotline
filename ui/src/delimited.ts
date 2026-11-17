/**
 * Comma- and tab-separated values (RFC 4180), read into rows of cells, for a
 * CSV or TSV file a teammate sent to open as a table.
 *
 * A field that starts with a quote runs to its closing quote, and may hold
 * the separator, line breaks and quotes written twice; a quote anywhere else
 * is an ordinary character. A line ends at LF, CRLF or a lone CR. A byte
 * order mark at the start is not part of the first cell, and a blank line is
 * no row. Rows keep the length they were written with: a ragged file is
 * drawn by its widest row.
 */
export function parseDelimited(text: string, separator: "," | "\t"): string[][] {
	const rows: string[][] = [];
	let row: string[] = [];
	let field = "";
	let fresh = true;
	let quoted = false;
	const endField = () => {
		row.push(field);
		field = "";
		fresh = true;
	};
	const endRow = () => {
		endField();
		if (row.length > 1 || row[0] !== "") rows.push(row);
		row = [];
	};
	for (let at = text.charCodeAt(0) === 0xfeff ? 1 : 0; at < text.length; at++) {
		const character = text[at]!;
		if (quoted) {
			if (character !== '"') field += character;
			else if (text[at + 1] === '"') {
				field += '"';
				at++;
			} else quoted = false;
		} else if (character === '"' && fresh) {
			quoted = true;
			fresh = false;
		} else if (character === separator) {
			endField();
		} else if (character === "\n" || character === "\r") {
			if (character === "\r" && text[at + 1] === "\n") at++;
			endRow();
		} else {
			field += character;
			fresh = false;
		}
	}
	if (!fresh || row.length > 0) endRow();
	return rows;
}
