import { useState } from "react";
import { workingState } from "../../activity";
import type { Attachment, ThreadId } from "../../generated/contract";
import { sends, useReturned, useSends } from "../../sends";
import { carry } from "../../serverFiles";
import { Composer } from "../Composer";
import type { Refill } from "../Conversation";

/**
 * The composer at a thread pane's foot: what is typed goes to the thread
 * through the window's sends, so a refused line comes back to this field
 * even when the person has gone elsewhere before the refusal came, and the
 * desk's word on it shows above the field.
 */
export function ThreadComposer({
	thread,
	name,
	working,
	prompt,
	interrupt,
}: {
	thread: ThreadId;
	name: string;
	/** A turn of the thread is running: the send key is Stop. */
	working: boolean;
	prompt(text: string, attachments?: Attachment[]): Promise<unknown>;
	interrupt(): Promise<unknown>;
}) {
	const conv = `${thread.kind}:${thread.key}`;
	const [refill, setRefill] = useState<Refill | null>(null);
	const threadSends = useSends(conv);
	useReturned(conv, (back) => setRefill({ text: back.text, attachments: back.attachments, nonce: back.nonce, merge: true }));
	return (
		<>
			{threadSends.notice !== null && (
				<p className="dock-note selectable" style={{ color: "var(--warn)" }}>
					{threadSends.notice}
				</p>
			)}
			<Composer
				embedded
				personaId={conv}
				name={name}
				state={workingState(working)}
				{...(refill !== null ? { refill } : {})}
				replyQuote={null}
				onSend={(text, attachments) => sends.send(conv, { text, attachments }, () => carry(attachments).then((carried) => prompt(text, carried)))}
				onCancel={() => void interrupt().catch(() => undefined)}
				onClearReply={() => undefined}
			/>
		</>
	);
}
