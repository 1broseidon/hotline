export const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

export const shortDate = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** Calendar days from `from` to `to`, not elapsed hours: 11pm and 1am are a day apart. */
export function daysBetween(from: Date, to: Date): number {
	const midnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
	return Math.round((midnight(to).getTime() - midnight(from).getTime()) / 86_400_000);
}
