/**
 * Colour for a block of code in a message: keywords, strings, numbers,
 * comments and the name of a function being called, for the languages a
 * teammate writes most. It is a scanner, not a parser, so it never knows more
 * than one token at a time; a language it does not know comes back as plain
 * lines. A highlighting library would carry grammars for hundreds of
 * languages to colour the same handful of things in a chat bubble.
 */

type RunKind = "keyword" | "string" | "number" | "comment" | "function";
/** A stretch of one line of code, and what it is when it is coloured. */
export type Run = { text: string; kind?: RunKind };

type Grammar = { comment: string; string: string; keywords: Set<string>; caseless?: boolean };

const SLASH_COMMENT = String.raw`//[^\n]*|/\*[\s\S]*?(?:\*/|$)`;
// A # starts a comment at a line's start or after a space, so `$#` and `a#b` stay code.
const HASH_COMMENT = String.raw`(?<!\S)#[^\n]*`;
const DASH_COMMENT = String.raw`--[^\n]*`;
const DOUBLE = String.raw`"(?:\\.|[^"\\\n])*"`;
const SINGLE = String.raw`'(?:\\.|[^'\\\n])*'`;
// One character in single quotes: a char literal, so Rust's lifetime `'a` is not a string.
const CHAR = String.raw`'(?:\\.|[^'\\\n])'`;
const BACKTICK = String.raw`\`(?:\\.|[^\`\\])*\``;
const TRIPLE = String.raw`"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)`;

const words = (list: string) => new Set(list.split(" "));
const LITERALS = "true false null";

const GRAMMARS: Record<string, Grammar> = {
	js: {
		comment: SLASH_COMMENT,
		string: [DOUBLE, SINGLE, BACKTICK].join("|"),
		keywords: words(
			`${LITERALS} undefined as async await break case catch class const continue default delete do else enum export extends finally for from function if implements import in instanceof interface let new of private protected public readonly return static super switch this throw try type typeof var void while yield`,
		),
	},
	py: {
		comment: HASH_COMMENT,
		string: [TRIPLE, DOUBLE, SINGLE].join("|"),
		keywords: words(
			"None True False and as assert async await break case class continue def del elif else except finally for from global if import in is lambda match nonlocal not or pass raise return self try while with yield",
		),
	},
	rust: {
		comment: SLASH_COMMENT,
		string: [DOUBLE, CHAR].join("|"),
		keywords: words(
			"true false as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while",
		),
	},
	go: {
		comment: SLASH_COMMENT,
		string: [DOUBLE, CHAR, BACKTICK].join("|"),
		keywords: words(
			"true false nil break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var",
		),
	},
	c: {
		comment: SLASH_COMMENT,
		string: [DOUBLE, CHAR].join("|"),
		keywords: words(
			`${LITERALS} nil abstract auto bool boolean break case catch char class const continue default do double else enum extends final float for fun func guard if implements import include int interface let long namespace new override package private protected public return static string struct switch this throw throws try using val var virtual void while`,
		),
	},
	sh: {
		comment: HASH_COMMENT,
		string: [DOUBLE, SINGLE].join("|"),
		keywords: words("case do done elif else esac exit export fi for function if in local readonly return set source then unset until while"),
	},
	ruby: {
		comment: HASH_COMMENT,
		string: [DOUBLE, SINGLE].join("|"),
		keywords: words("true false nil and begin class def do else elsif end ensure if in module next not or raise require rescue return self then unless until when while yield"),
	},
	sql: {
		comment: DASH_COMMENT,
		string: SINGLE,
		caseless: true,
		keywords: words(
			"all alter and as asc between by case create delete desc distinct drop else end exists from group having in index inner insert into is join key left like limit not null on or order outer primary references returning right select set table then union update values when where with",
		),
	},
	json: { comment: SLASH_COMMENT, string: DOUBLE, keywords: words(LITERALS) },
	yaml: { comment: HASH_COMMENT, string: [DOUBLE, SINGLE].join("|"), keywords: words(`${LITERALS} yes no`) },
	css: { comment: SLASH_COMMENT, string: [DOUBLE, SINGLE].join("|"), keywords: words("important") },
};

const ALIASES: Record<string, string> = {
	javascript: "js", jsx: "js", mjs: "js", cjs: "js", ts: "js", typescript: "js", tsx: "js",
	python: "py", python3: "py",
	rs: "rust",
	golang: "go",
	cpp: "c", "c++": "c", h: "c", hpp: "c", java: "c", cs: "c", csharp: "c", kotlin: "c", kt: "c", swift: "c", php: "c", dart: "c", scala: "c",
	bash: "sh", zsh: "sh", shell: "sh", console: "sh", shellscript: "sh",
	rb: "ruby",
	psql: "sql", postgres: "sql", postgresql: "sql", mysql: "sql", sqlite: "sql",
	jsonc: "json", json5: "json",
	yml: "yaml", toml: "yaml", ini: "yaml",
	scss: "css",
};

const NUMBER = String.raw`\b(?:0x[0-9a-fA-F]+|\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?)\b`;
const WORD = String.raw`[A-Za-z_$][\w$]*`;

/** The code as lines of runs, coloured when the language is one of the known ones. */
export function highlight(code: string, language: string | undefined): Run[][] {
	const name = language?.toLowerCase() ?? "";
	const grammar = GRAMMARS[ALIASES[name] ?? name];
	if (grammar === undefined) return code.split("\n").map((line) => (line === "" ? [] : [{ text: line }]));
	const scanner = new RegExp(`(${grammar.comment})|(${grammar.string})|(${NUMBER})|(${WORD})`, "g");
	const runs: Run[] = [];
	let at = 0;
	for (const match of code.matchAll(scanner)) {
		const index = match.index;
		if (index > at) runs.push({ text: code.slice(at, index) });
		const [text, comment, string, number, word] = match;
		const kind: RunKind | undefined =
			comment !== undefined ? "comment"
			: string !== undefined ? "string"
			: number !== undefined ? "number"
			: word !== undefined && grammar.keywords.has(grammar.caseless === true ? word.toLowerCase() : word) ? "keyword"
			: word !== undefined && /^\s*\(/.test(code.slice(index + text.length, index + text.length + 8)) ? "function"
			: undefined;
		runs.push(kind === undefined ? { text } : { text, kind });
		at = index + text.length;
	}
	if (at < code.length) runs.push({ text: code.slice(at) });
	return intoLines(runs);
}

/** Runs cut at each newline, so a comment or string spanning lines colours each line it covers. */
function intoLines(runs: Run[]): Run[][] {
	const lines: Run[][] = [[]];
	for (const run of runs) {
		run.text.split("\n").forEach((piece, i) => {
			if (i > 0) lines.push([]);
			if (piece !== "") lines[lines.length - 1]!.push(run.kind === undefined ? { text: piece } : { text: piece, kind: run.kind });
		});
	}
	return lines;
}
