/**
 * Environment variables as a person writes them, one `KEY=value` per line,
 * rather than as JSON. Blank lines and `#` comments are skipped; a value may
 * hold `=` and spaces, and is taken as written.
 */
export function envToLines(env: Record<string, string> | undefined): string {
	return Object.entries(env ?? {})
		.map(([key, value]) => `${key}=${value}`)
		.join("\n");
}

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** The variables in `text`, or the first line that is not one, in words. */
export function linesToEnv(text: string): { env: Record<string, string> } | { error: string } {
	const env: Record<string, string> = {};
	for (const [index, raw] of text.split("\n").entries()) {
		const line = raw.trim();
		if (line === "" || line.startsWith("#")) continue;
		const at = line.indexOf("=");
		const key = at < 0 ? line : line.slice(0, at).trim();
		if (at < 0 || !NAME.test(key)) return { error: `Line ${index + 1}: write each variable as NAME=value.` };
		env[key] = line.slice(at + 1).trim();
	}
	return { env };
}
