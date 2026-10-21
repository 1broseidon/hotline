import { useCallback, useEffect, useSyncExternalStore } from "react";
import { activeDeskId, wireFor } from "./desks";
import { useRoomReadEpoch } from "./room";
import type { Commands, Wire } from "./wire";

// Only room-wide picker metadata belongs here. Credentials and persona status
// deliberately have no entry point into this cache.
type Read = "backends.list" | "models.efforts" | "capabilities.options" | "voice.status";
type Entry = { value?: unknown; pending?: Promise<void> | undefined; listeners: Set<() => void> };
type Cache = { epoch: object; entries: Map<string, Entry> };
const caches = new WeakMap<Wire, Cache>();

/** Share both in-flight and settled reads, scoped to this connection and room epoch. */
export function useRoomRead<N extends Read>(name: N, modelId = "", enabled = true): Commands[N]["result"] | undefined {
	const epoch = useRoomReadEpoch();
	const connection = wireFor(activeDeskId() ?? "");
	let cache = caches.get(connection);
	if (cache === undefined || cache.epoch !== epoch) {
		cache = { epoch, entries: new Map() };
		caches.set(connection, cache);
	}
	const entries = cache.entries;
	const key = JSON.stringify([name, modelId]);
	let entry = entries.get(key);
	if (entry === undefined) {
		entry = { listeners: new Set() };
		entries.set(key, entry);
	}
	const subscribe = useCallback((listener: () => void) => {
		entry.listeners.add(listener);
		return () => { entry.listeners.delete(listener); };
	}, [entry]);
	const value = useSyncExternalStore(subscribe, () => entry.value);
	useEffect(() => {
		if (!enabled || !epoch.ready || entry.value !== undefined) return;
		if (entry.pending === undefined) {
			const command: Read = name;
			const request = command === "models.efforts"
				? connection.command(command, { modelId })
				: connection.command(command, {});
			entry.pending = request.then(
				(value) => {
					entry.value = value;
					for (const listener of entry.listeners) listener();
				},
				() => { entry.pending = undefined; },
			);
		}
	}, [connection, enabled, entry, epoch, modelId, name]);
	return enabled && epoch.ready ? value as Commands[N]["result"] | undefined : undefined;
}
