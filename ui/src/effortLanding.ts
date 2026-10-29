/*
 * Which effort a model switch lands on, as the core chooses it
 * (`crates/hotline-core/src/wire/commands/models/effort.rs`, which is the
 * truth): drawn in the menu, dimmed, while a running teammate's switch is
 * on its way, so the new model's tabs show at once. The core's answer
 * replaces it.
 *
 * In order: the teammate's last for the new model; else the desk's; else
 * the current effort where the model has it; else the highest level it has
 * that is not above the current one, never a step up; else its default.
 * Auto ("default") carries over only to a model that has it and is never
 * ranked.
 */

/** Effort levels by how much they think, for levels two models' lists cannot place. */
const LADDER = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

const isAuto = (level: string) => level.toLowerCase() === "default" || level.toLowerCase() === "auto";

export type Switch = {
	/** The new model's levels in its own order. */
	levels: string[];
	default?: string | undefined;
	teammate?: string | undefined;
	desk?: string | undefined;
	current?: string | undefined;
	/** The levels of the model in use, in its order. */
	currentLevels: string[];
};

export function landsOn(switched: Switch): string | undefined {
	const { levels, current } = switched;
	if (levels.length === 0) return undefined;
	const offered = (level: string | undefined): level is string => level !== undefined && levels.includes(level);
	const kept = [switched.teammate, switched.desk, current].find(offered);
	if (kept !== undefined) return kept;
	if (current !== undefined && !isAuto(current)) {
		const stepped = levels.filter((level) => !isAuto(level) && notAbove(level, current, switched.currentLevels)).at(-1);
		if (stepped !== undefined) return stepped;
	}
	return offered(switched.default) ? switched.default : levels[0];
}

/** By the current model's own order when it lists both, else by the ladder; never when neither can place them. */
function notAbove(level: string, current: string, currentLevels: string[]): boolean {
	const own = currentLevels.filter((one) => !isAuto(one));
	if (own.includes(level) && own.includes(current)) return own.indexOf(level) <= own.indexOf(current);
	if (LADDER.includes(level) && LADDER.includes(current)) return LADDER.indexOf(level) <= LADDER.indexOf(current);
	return false;
}
