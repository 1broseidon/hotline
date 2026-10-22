import { type KeyboardEvent, useId, useState } from "react";
import type { AccountView, SetupFrom } from "../../generated/contract";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { nextAccountName, SETUP_CATEGORIES } from "./words";

/**
 * It only creates the account; signing it in is the caller's, which is why
 * the email is handed up rather than sent. There is no <form> here: New
 * teammate nests this in its own, and a form may not hold another.
 */
export function AddAccountForm({
	backendId,
	agentName,
	takesEmail,
	taken,
	compact = false,
	onAdded,
	onRefused,
	onCancel,
}: {
	backendId: string;
	agentName: string;
	takesEmail: boolean;
	taken: string[];
	compact?: boolean;
	onAdded(account: AccountView, email: string | undefined): void;
	onRefused(): void;
	onCancel(): void;
}) {
	const id = useId();
	const [name, setName] = useState("");
	const [email, setEmail] = useState("");
	const [startWith, setStartWith] = useState<SetupFrom>("system_default");
	const [customize, setCustomize] = useState(false);
	const [ticked, setTicked] = useState<Set<string>>(() => new Set(SETUP_CATEGORIES.map((one) => one.id)));
	const [choosing, setChoosing] = useState(false);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const placeholder = nextAccountName(taken);
	const mine = startWith === "system_default";

	const add = async () => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		// Every kind ticked is the same as not saying, so only a narrower choice is sent.
		const chosen = SETUP_CATEGORIES.filter((one) => ticked.has(one.id)).map((one) => one.id);
		const narrowed = mine && chosen.length < SETUP_CATEGORIES.length;
		try {
			const created = await wire.command("accounts.create", {
				backendId,
				name: name.trim() || placeholder,
				startWith,
				...(narrowed ? { categories: chosen } : {}),
			});
			onAdded(created.account, email.trim() || undefined);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			onRefused();
		} finally {
			setBusy(false);
		}
	};

	const flip = (category: string) =>
		setTicked((was) => {
			const next = new Set(was);
			if (next.has(category)) next.delete(category);
			else next.add(category);
			return next;
		});

	const addOnEnter = (event: KeyboardEvent<HTMLInputElement>) => {
		if (event.key !== "Enter") return;
		event.preventDefault();
		void add();
	};

	const choice = (
		<>
			<span className="label" id={`${id}-start`}>
				Start with
			</span>
			<div className="grouped" role="radiogroup" aria-labelledby={`${id}-start`}>
				<label className="group-row group-row-choice">
					<input type="radio" className="radio" name={`${id}-start`} checked={mine} disabled={busy} onChange={() => setStartWith("system_default")} />
					<span className="group-row-text">
						<span className="group-row-title">My {agentName} setup</span>
						<span className="group-row-detail">A copy of your own {agentName} setup, without the sign-in, keys or history</span>
					</span>
				</label>
				<label className="group-row group-row-choice">
					<input type="radio" className="radio" name={`${id}-start`} checked={!mine} disabled={busy} onChange={() => setStartWith("empty")} />
					<span className="group-row-text">
						<span className="group-row-title">Empty</span>
						<span className="group-row-detail">{agentName}'s defaults; add what this account needs afterwards</span>
					</span>
				</label>
			</div>
			{mine && (
				<button type="button" className="control btn-quiet self-start" aria-expanded={customize} onClick={() => setCustomize((was) => !was)}>
					{customize ? "Hide what is copied" : "Customize what is copied…"}
				</button>
			)}
			{mine && customize && (
				<div className="grouped">
					{SETUP_CATEGORIES.map((category) => (
						<label key={category.id} className="group-row group-row-choice">
							<input type="checkbox" className="check" checked={ticked.has(category.id)} disabled={busy} onChange={() => flip(category.id)} />
							<span className="group-row-text">
								<span className="group-row-title">{category.choice}</span>
							</span>
						</label>
					))}
				</div>
			)}
		</>
	);

	const keys = (
		<div className="flex justify-end gap-2">
			<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
				Cancel
			</button>
			<button type="button" className="control btn btn-primary" disabled={busy} onClick={() => void add()}>
				{busy ? "Adding…" : "Add and sign in"}
			</button>
		</div>
	);

	if (compact) {
		return (
			<div className="group-row flex-col items-stretch gap-2">
				<input
					className="field"
					aria-label="New account name"
					placeholder={placeholder}
					value={name}
					disabled={busy}
					onChange={(event) => setName(event.target.value)}
					onKeyDown={addOnEnter}
				/>
				{takesEmail && (
					<input
						type="email"
						className="field"
						aria-label={`Email of the ${agentName} account (optional)`}
						placeholder="Email (optional)"
						autoComplete="off"
						value={email}
						disabled={busy}
						onChange={(event) => setEmail(event.target.value)}
						onKeyDown={addOnEnter}
					/>
				)}
				{choosing ? (
					choice
				) : (
					<p className="text-sm text-ink-3">
						Starts with your {agentName} setup ·{" "}
						<button type="button" className="text-accent-ink hover:underline" aria-expanded={false} onClick={() => setChoosing(true)}>
							Change or customize
						</button>
					</p>
				)}
				{keys}
				{refusal !== null && <Refusal message={refusal} />}
			</div>
		);
	}

	return (
		<section>
			<h3 className="group-title">What a new account starts with</h3>
			<p className="group-hint mt-0 mb-2">
				Chosen once, here. After that the account is edited on its own in its Setup, where it can also copy from your setup again.
			</p>
			<div className="grouped">
				<div className="group-row flex-col items-stretch gap-1">
					<label className="label" htmlFor={`${id}-name`}>
						Name
					</label>
					<input
						id={`${id}-name`}
						className="field"
						placeholder={placeholder}
						value={name}
						disabled={busy}
						onChange={(event) => setName(event.target.value)}
						onKeyDown={addOnEnter}
					/>
				</div>
				{takesEmail && (
					<div className="group-row flex-col items-stretch gap-1">
						<label className="label" htmlFor={`${id}-email`}>
							Email of the {agentName} account <span className="font-normal text-ink-4">(optional)</span>
						</label>
						<input
							id={`${id}-email`}
							type="email"
							className="field"
							placeholder="name@example.com"
							autoComplete="off"
							value={email}
							disabled={busy}
							onChange={(event) => setEmail(event.target.value)}
							onKeyDown={addOnEnter}
						/>
						<p className="hint">Fills in the email on {agentName}'s sign-in page, so the right account is used. Hotline does not keep it.</p>
					</div>
				)}
				<div className="group-row flex-col items-stretch gap-2">{choice}</div>
				<div className="group-row justify-end">{keys}</div>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}
