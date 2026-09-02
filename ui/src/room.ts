import { useEffect, useMemo, useState } from "react";
import type { ScheduledJob } from "./generated/contract";
import { mcpServersFrom, type McpServer } from "./mcp";
import { wire } from "./wire";

/**
 * The room stream, as the window reads it.
 *
 * Settings and jobs are both events on `room`, folded by id. A delete is a
 * tombstone of the same kind, so a snapshot already has the latest word and
 * a live event either replaces the row or takes it away. Calling
 * `schedule.list` would be a second copy of the same fold, and would still
 * have to listen for room events to stay true.
 */

/** A chapter closes after this many idle hours unless someone has set another. */
export const DEFAULT_IDLE_HOURS = 8;

/**
 * One line on the room stream. Settings are `kind: "setting"` (`id` the key,
 * `value` the value). Jobs are `kind: "schedule"`; that is the stream's
 * kind, not the job's, and a loop is recovered from `every`.
 */
type RoomItem = {
	kind?: string;
	id?: string;
	value?: unknown;
	deleted?: boolean;
	personaId?: string;
	when?: number;
	every?: number;
	prompt?: string;
	quiet?: boolean;
	nextAt?: number;
	createdAt?: number;
};

/**
 * Subscribe to the room and fold its setting events over the defaults. A
 * reconnect delivers a fresh snapshot, so the map is replaced rather than
 * merged.
 */
export function useRoomSettings(): {
	chapterIdleHours: number;
	defaultBackendId: string;
	defaultModelId: string | null;
	lastModelId: string | null;
	mcpServers: McpServer[];
	enabledModels: Record<string, string[]>;
} {
	const [events, setEvents] = useState<Map<string, RoomItem>>(new Map());

	useEffect(() => {
		return wire.subscribe<RoomItem>("room", {
			snapshot: (items) => setEvents(takeKind(items, "setting")),
			event: (item) => {
				if (item.kind !== "setting" || typeof item.id !== "string") return;
				const id = item.id;
				setEvents((known) => {
					const next = new Map(known);
					next.set(id, item);
					return next;
				});
			},
		});
	}, []);

	const enabledModels = useMemo(
		() => enabledModelsSetting(events.get("enabledModels")),
		[events],
	);

	return {
		chapterIdleHours: numberSetting(events.get("chapterIdleHours"), DEFAULT_IDLE_HOURS),
		defaultBackendId: stringSetting(events.get("defaultBackendId"), "pi"),
		defaultModelId: optionalStringSetting(events.get("defaultModelId")),
		lastModelId: optionalStringSetting(events.get("lastModelId")),
		mcpServers: listSetting(events.get("mcpServers")),
		enabledModels,
	};
}

/**
 * The jobs still waiting to fire, soonest first. Folded from the room the
 * same way settings are, so the header pill and the teammate list see one
 * picture.
 */
export function useRoomJobs(): ScheduledJob[] {
	const [jobs, setJobs] = useState<ScheduledJob[]>([]);

	useEffect(() => {
		return wire.subscribe<RoomItem>("room", {
			snapshot: (items) => setJobs(takeJobs(items)),
			event: (item) => {
				if (item.kind !== "schedule" || typeof item.id !== "string") return;
				const id = item.id;
				setJobs((known) => {
					const without = known.filter((job) => job.id !== id);
					if (item.deleted) return without;
					const job = jobFromEvent(item);
					return job ? sortJobs([...without, job]) : without;
				});
			},
		});
	}, []);

	return jobs;
}

/** How far away a fire is, in the smallest unit that still reads. */
export function nextText(at: number, now = Date.now()): string {
	const delta = at - now;
	if (delta <= 0) return "now";
	if (delta < 60_000) return `in ${Math.max(1, Math.round(delta / 1_000))}s`;
	if (delta < 3_600_000) return `in ${Math.round(delta / 60_000)}m`;
	if (delta < 86_400_000) return `in ${Math.round(delta / 3_600_000)}h`;
	return `in ${Math.round(delta / 86_400_000)}d`;
}

/** An interval as a person would say it. */
export function durationText(ms: number): string {
	if (ms < 60_000) return `${Math.round(ms / 1_000)}s`;
	if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`;
	if (ms < 86_400_000) return `${Math.round(ms / 3_600_000)}h`;
	return `${Math.round(ms / 86_400_000)}d`;
}

/** The prompt's first line, which is what the list has room to show. */
export function firstLine(prompt: string): string {
	const line = prompt.split(/\r?\n/, 1)[0] ?? "";
	return line.trim() === "" ? prompt.trim() : line;
}

function takeJobs(items: RoomItem[]): ScheduledJob[] {
	const jobs: ScheduledJob[] = [];
	for (const item of items) {
		const job = jobFromEvent(item);
		if (job) jobs.push(job);
	}
	return sortJobs(jobs);
}

/**
 * The event's `kind` is always `schedule`. A loop is the job that carries
 * `every`; a one-shot is everything else that still reads as a job. A
 * half-written line is skipped, the same as a teammate that does not read
 * as a persona.
 */
function jobFromEvent(item: RoomItem): ScheduledJob | null {
	if (item.kind !== "schedule" || item.deleted) return null;
	if (typeof item.id !== "string" || item.id === "") return null;
	if (typeof item.personaId !== "string" || item.personaId === "") return null;
	if (typeof item.prompt !== "string" || item.prompt.trim() === "") return null;
	if (typeof item.nextAt !== "number" || typeof item.createdAt !== "number") return null;
	const every = typeof item.every === "number" ? item.every : undefined;
	const when = typeof item.when === "number" ? item.when : undefined;
	return {
		id: item.id,
		personaId: item.personaId,
		kind: every !== undefined ? "loop" : "schedule",
		...(when !== undefined ? { when } : {}),
		...(every !== undefined ? { every } : {}),
		prompt: item.prompt,
		...(item.quiet === true ? { quiet: true } : {}),
		nextAt: item.nextAt,
		createdAt: item.createdAt,
	};
}

function sortJobs(jobs: ScheduledJob[]): ScheduledJob[] {
	return jobs.slice().sort((a, b) => a.nextAt - b.nextAt || a.id.localeCompare(b.id));
}

function takeKind(items: RoomItem[], kind: string): Map<string, RoomItem> {
	const map = new Map<string, RoomItem>();
	for (const item of items) {
		if (item.kind === kind && typeof item.id === "string") map.set(item.id, item);
	}
	return map;
}

function listSetting(event: RoomItem | undefined): McpServer[] {
	if (!event || event.deleted) return [];
	return mcpServersFrom(event.value);
}

function numberSetting(event: RoomItem | undefined, fallback: number): number {
	if (!event || event.deleted || typeof event.value !== "number") return fallback;
	return event.value;
}

function stringSetting(event: RoomItem | undefined, fallback: string): string {
	if (!event || event.deleted || typeof event.value !== "string") return fallback;
	return event.value;
}

function optionalStringSetting(event: RoomItem | undefined): string | null {
	if (!event || event.deleted || typeof event.value !== "string") return null;
	return event.value;
}

/**
 * A provider absent from the object shows every model. A value that is not
 * an object, or an entry that is not an array of strings, reads as absent —
 * the same rule the core uses, so a bad setting costs its own filter.
 */
function enabledModelsSetting(event: RoomItem | undefined): Record<string, string[]> {
	if (!event || event.deleted || event.value === null || typeof event.value !== "object" || Array.isArray(event.value)) {
		return {};
	}
	const out: Record<string, string[]> = {};
	for (const [provider, value] of Object.entries(event.value as Record<string, unknown>)) {
		if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) continue;
		out[provider] = value;
	}
	return out;
}
