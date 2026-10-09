import { useEffect, useState } from "react";
import type { BudgetKind, CapabilityOptions, SpendingKind } from "../generated/contract";
import { budgetsPatch, limitText, limitsOf, type Limits } from "../budgets";
import { Fold, toggled } from "../ui/Fold";
import { Refusal } from "../ui/Refusal";
import { parseCap, usd } from "../useFor";
import { wire } from "../wire";

type Spending = CapabilityOptions["spending"];

const BUDGETS: Record<BudgetKind, string> = {
	chat: "Chat",
	voice: "Voice",
	images: "Images",
};

const LINES: Record<SpendingKind, string> = {
	teammates: "Teammates",
	callAssistant: "Call assistant",
	transcription: "Transcription",
	speech: "Speech",
};

/**
 * What paid use has cost, as three budgets: Chat, Voice and Images, under
 * Use for on Providers. Only a key billed per use counts; a subscription or
 * a model on this computer costs nothing here. Each budget is a fold row,
 * spend and limit, opened to see what it went on and to set its limits.
 * A budget has no limit until one is set, and 0 turns its paid use off.
 */
export function Budgets({ spending, onChanged }: { spending: Spending; onChanged(): void }) {
	const [open, setOpen] = useState<BudgetKind | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);

	const write = (kind: BudgetKind, next: Limits) => {
		setRefusal(null);
		wire
			.command("settings.update", { patch: { spending: budgetsPatch(spending, kind, next) } })
			.then(onChanged)
			.catch((error: Error) => setRefusal(error.message));
	};

	return (
		<section>
			<h3 className="group-title">Budgets</h3>
			<div className="grouped">
				{spending.budgets.map((budget) => (
					<Fold
						key={budget.kind}
						title={BUDGETS[budget.kind]}
						value={
							<>
								{usd(budget.spentDayUsd)} today · {usd(budget.spentMonthUsd)} this month
								<span className="text-ink-3"> · {limitText(budget)}</span>
							</>
						}
						open={open === budget.kind}
						onToggle={() => setOpen((was) => toggled(was, budget.kind))}
					>
						{budget.lines.map((line) => (
							<div key={line.kind} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{LINES[line.kind]}</span>
								</span>
								<span className="shrink-0 text-sm text-ink-3">
									{usd(line.dayUsd)} today · {usd(line.monthUsd)} this month
								</span>
							</div>
						))}
						<div className="group-row">
							<span className="group-row-text">
								<span className="group-row-title">Limits</span>
							</span>
							<span className="flex shrink-0 items-center gap-3">
								<Cap label={`${BUDGETS[budget.kind]} daily limit`} value={limitsOf(budget).dayUsd} unit="a day" onCommit={(dayUsd) => write(budget.kind, { ...limitsOf(budget), dayUsd })} />
								<Cap label={`${BUDGETS[budget.kind]} monthly limit`} value={limitsOf(budget).monthUsd} unit="a month" onCommit={(monthUsd) => write(budget.kind, { ...limitsOf(budget), monthUsd })} />
							</span>
						</div>
					</Fold>
				))}
			</div>
			<p className="group-hint">
				{spending.unavailable === undefined
					? "Only keys billed per use count. Subscriptions and models on this computer are free."
					: "Some spending could not be read, so these totals are short."}
			</p>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** A limit typed in, saved on Enter or when you leave it. Empty is no limit; 0 turns paid use off. */
function Cap({ label, value, unit, onCommit }: { label: string; value: number | null; unit: string; onCommit(usd: number | null): void }) {
	const shown = (amount: number | null) => (amount === null ? "" : amount.toFixed(2));
	const [text, setText] = useState(shown(value));

	useEffect(() => {
		setText(shown(value));
	}, [value]);

	const commit = () => {
		const next = text.trim() === "" ? null : parseCap(text);
		if (next === null && text.trim() !== "") {
			setText(shown(value));
			return;
		}
		setText(shown(next));
		if (next !== value) onCommit(next);
	};

	return (
		<label className="flex items-center gap-1 text-sm text-ink-3">
			<span>$</span>
			<input
				type="text"
				inputMode="decimal"
				className="field w-20 text-right"
				aria-label={label}
				placeholder="No limit"
				value={text}
				onChange={(event) => setText(event.target.value)}
				onBlur={commit}
				onKeyDown={(event) => {
					if (event.key === "Enter") event.currentTarget.blur();
				}}
			/>
			{unit}
		</label>
	);
}
