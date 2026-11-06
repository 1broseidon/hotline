import { Fragment } from "react";

/** The most steps a chain is read as one line; a longer flow keeps its drawing. */
const LONGEST = 8;

/**
 * The steps of a diagram that is only a straight line, `A --> B --> C`, in
 * order, by their labels; `null` for anything else: a branch, a loop, an
 * edge with a label, a subgraph, a style, or a flow of one step.
 */
export function chainOf(source: string): string[] | null {
	const lines = source
		.split("\n")
		.map((line) => line.trim().replace(/;$/, ""))
		.filter((line) => line !== "" && !line.startsWith("%%"));
	const head = lines.shift();
	if (head === undefined || !/^(flowchart|graph)\s+(LR|RL|TD|TB|BT)$/i.test(head)) return null;
	const node = /^([A-Za-z0-9_]+)(?:\[([^\]]*)\]|\(\(([^)]*)\)\)|\(([^)]*)\)|\{([^}]*)\})?$/;
	const labels = new Map<string, string>();
	const next = new Map<string, string>();
	const into = new Map<string, number>();
	for (const line of lines) {
		const parts = line.split(/\s*-->\s*/);
		if (parts.length < 2) return null;
		let before: string | null = null;
		for (const part of parts) {
			const found = node.exec(part);
			if (found === null) return null;
			const id = found[1]!;
			const label = found[2] ?? found[3] ?? found[4] ?? found[5];
			if (label !== undefined) labels.set(id, label.replace(/^"(.*)"$/, "$1").trim());
			if (!labels.has(id)) labels.set(id, id);
			if (before !== null) {
				if (next.has(before)) return null;
				next.set(before, id);
				into.set(id, (into.get(id) ?? 0) + 1);
			}
			before = id;
		}
	}
	const starts = [...labels.keys()].filter((id) => !into.has(id));
	if (starts.length !== 1 || [...into.values()].some((count) => count > 1)) return null;
	const order: string[] = [];
	for (let at: string | undefined = starts[0]; at !== undefined; at = next.get(at)) {
		if (order.includes(at)) return null;
		order.push(at);
	}
	if (order.length !== labels.size || order.length < 2 || order.length > LONGEST) return null;
	return order.map((id) => labels.get(id)!);
}

/**
 * A diagram that is only a straight line, said as one: its steps as quiet
 * chips with arrows between, wrapping like words. A drawing of five boxes
 * in a row says no more than this, and is harder to read at a bubble's size.
 */
export function FlowChain({ steps }: { steps: string[] }) {
	return (
		<p className="flow-chain" role="img" aria-label={steps.join(", then ")}>
			{steps.map((step, index) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: a step is its place in the chain.
				<Fragment key={index}>
					{index > 0 && (
						<span className="flow-arrow" aria-hidden="true">
							→
						</span>
					)}
					<span className="flow-step">{step}</span>
				</Fragment>
			))}
		</p>
	);
}
