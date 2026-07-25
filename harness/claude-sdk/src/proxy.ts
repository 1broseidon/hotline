/**
 * In-process MCP proxy: exposes the hotline child's tools to the Agent SDK
 * session as server "hotline" (tool names become mcp__hotline__reply etc.).
 *
 * Why a proxy instead of pointing mcpServers at `hotline run` directly
 * (spec §0): the SDK's MCP client has no hook for custom server→client
 * notifications, so the inbound channel push would be silently dropped, and
 * `hotline run` holds an exclusive box lease so a second instance cannot run.
 * The harness owns the child; this proxy forwards tools/list + tools/call.
 *
 * Rules (spec §4):
 *   - Tool schemas pass through VERBATIM (raw JSON schemas via low-level
 *     request handlers; no zod re-modelling).
 *   - Child isError results pass through as isError results — never thrown.
 *   - The tool list is a snapshot updated on each onReady; the client is read
 *     live per call so tool calls keep working across child respawns.
 *     NOTE: the Agent SDK caches tools/list at connect, so a respawn with a
 *     CHANGED tool set won't refresh the model's list (calls still route
 *     through the live client). Cosmetic, pi-precedent.
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { ListToolsRequestSchema, CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import {
  JsonRpcClient,
  mcpCallTool,
  timeoutsFromEnv,
  type McpTool,
} from "@1broseidon/hotline-harness-core/jsonrpc";

export interface ProxyDeps {
  /** The live child client, or null while the child is down/respawning. */
  getClient: () => JsonRpcClient | null;
  /** The last tools/list snapshot captured at onReady. */
  getTools: () => McpTool[];
}

export interface SdkMcpServerConfig {
  type: "sdk";
  name: string;
  instance: McpServer;
}

export const CHANNEL_DOWN_MESSAGE =
  "hotline channel is down (reconnecting). The message was not delivered; retry shortly.";

export function buildHotlineProxy(deps: ProxyDeps): SdkMcpServerConfig {
  const timeouts = timeoutsFromEnv("HOTLINE_SDK");
  const server = new McpServer({ name: "hotline", version: "0.1.0" });
  // Low-level handlers need the capability declared explicitly (the high-level
  // tool() helper would do this implicitly, but it forces zod schemas).
  server.server.registerCapabilities({ tools: {} });
  server.server.setRequestHandler(ListToolsRequestSchema, async () => ({
    // Child's tools verbatim: name/description/inputSchema untouched.
    tools: deps.getTools() as never,
  }));
  const channelDown = () =>
    ({
      content: [{ type: "text" as const, text: CHANNEL_DOWN_MESSAGE }],
      isError: true,
    }) as never;
  server.server.setRequestHandler(CallToolRequestSchema, async (req) => {
    const client = deps.getClient();
    if (!client) return channelDown();
    try {
      return (await mcpCallTool(
        client,
        req.params.name,
        req.params.arguments ?? {},
        timeouts.callTimeoutMs,
      )) as never;
    } catch {
      // The getClient() null check above only covers a child that was ALREADY
      // down. The real race is the child exiting DURING the call: the in-flight
      // request rejects (closed transport, timeout, abort), and an uncaught
      // rejection here surfaces to the model as an MCP protocol error rather
      // than the documented retryable isError. A protocol error reads as "this
      // tool is broken"; the isError text says "retry shortly", which is the
      // truth — the supervisor is already respawning the child.
      //
      // Catching everything is deliberate. A hotline tool reports its own
      // failures through isError in the RESULT (mcpCallTool resolves, it does
      // not throw), so a throw reaching here is always a transport condition,
      // never the tool disagreeing with its arguments.
      return channelDown();
    }
  });
  return { type: "sdk", name: "hotline", instance: server };
}
