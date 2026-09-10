import { useEffect, useState } from "react";
import { wire } from "./wire";

/**
 * MCP servers as the settings pane writes them.
 *
 * The generated contract has the grant (`McpPolicy`) but not this list: the
 * core stores it as the setting key `mcpServers`, and another branch owns the
 * reader. The window writes the previous Toad's shape so that reader, and an
 * imported data directory, see the same entries.
 */

export type McpStdioServer = {
	id: string;
	type: "stdio";
	name: string;
	command: string;
	args: string[];
	credentialRef?: string;
	launchValuesPending?: boolean;
	env?: Record<string, string>;
};

/**
 * A new HTTP server is always `{ mode: "none" }`. An entry that already
 * carries another auth object is kept as written, so editing a neighbour
 * does not strip it.
 */
export type McpHttpAuth = { mode: "none" } | { mode: string; [key: string]: unknown };

export type McpHttpServer = {
	id: string;
	type: "http";
	name: string;
	url: string;
	urlNeedsRepair?: boolean;
	auth: McpHttpAuth;
};

export type McpServer = McpStdioServer | McpHttpServer;

/** One line of the room stream. Settings are `kind: "setting"`. */
type RoomItem = {
	kind?: string;
	id?: string;
	value?: unknown;
	deleted?: boolean;
};

/**
 * The app's MCP servers, folded from the room. A reconnect replaces the
 * snapshot rather than merging, the same as every other room reader.
 */
export function useMcpServers(): McpServer[] {
	const [servers, setServers] = useState<McpServer[]>([]);

	useEffect(() => {
		return wire.subscribe<RoomItem>("room", {
			snapshot: (items) => setServers(mcpServersFrom(settingValue(items, "mcpServers"))),
			event: (item) => {
				if (item.kind !== "setting" || item.id !== "mcpServers") return;
				setServers(item.deleted ? [] : mcpServersFrom(item.value));
			},
		});
	}, []);

	return servers;
}

/** The command line, or the URL — whichever the list and the grant show. */
export function mcpServerDetail(server: McpServer): string {
	return server.type === "stdio" ? [server.command, ...server.args].join(" ") : server.urlNeedsRepair ? "Re-enter this source’s URL" : server.url;
}

/**
 * Read the stored list. A half-written entry is dropped rather than failing
 * the rest, because a person can edit this value and one bad row should not
 * hide every tool.
 */
export function mcpServersFrom(value: unknown): McpServer[] {
	if (!Array.isArray(value)) return [];
	const servers: McpServer[] = [];
	for (const raw of value) {
		const server = readServer(raw);
		if (server) servers.push(server);
	}
	return servers;
}

function settingValue(items: RoomItem[], key: string): unknown {
	for (const item of items) {
		if (item.kind === "setting" && item.id === key && !item.deleted) return item.value;
	}
	return undefined;
}

function readServer(value: unknown): McpServer | null {
	if (!value || typeof value !== "object" || Array.isArray(value)) return null;
	const raw = value as Record<string, unknown>;
	const id = typeof raw.id === "string" && raw.id ? raw.id : null;
	const name = typeof raw.name === "string" ? raw.name.trim() : "";
	if (!id || !name) return null;

	if (raw.type === "http") {
		const url = typeof raw.url === "string" ? raw.url.trim() : "";
		if (!url && raw.urlNeedsRepair !== true) return null;
		return { id, type: "http", name, url, auth: readAuth(raw.auth),
			...(raw.urlNeedsRepair === true ? { urlNeedsRepair: true } : {}) };
	}

	if (raw.type !== "stdio" && raw.type !== undefined) return null;
	const command = typeof raw.command === "string" ? raw.command.trim() : "";
	if (!command) return null;
	const args = Array.isArray(raw.args)
		? raw.args.filter((arg): arg is string => typeof arg === "string")
		: [];
	const env = isStringMap(raw.env) ? raw.env : undefined;
	return { id, type: "stdio", name, command, args, ...(env ? { env } : {}),
		...(typeof raw.credentialRef === "string" ? { credentialRef: raw.credentialRef } : {}),
		...(raw.launchValuesPending === true ? { launchValuesPending: true } : {}) };
}

function readAuth(value: unknown): McpHttpAuth {
	if (!value || typeof value !== "object" || Array.isArray(value)) return { mode: "none" };
	const raw = value as Record<string, unknown>;
	if (raw.mode === "none") return { mode: "none" };
	if (typeof raw.mode === "string") return raw as McpHttpAuth;
	return { mode: "none" };
}

function isStringMap(value: unknown): value is Record<string, string> {
	if (!value || typeof value !== "object" || Array.isArray(value)) return false;
	return Object.values(value).every((item) => typeof item === "string");
}
