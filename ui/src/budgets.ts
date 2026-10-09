import type { CapabilitySpending, SpendingBudget, BudgetKind } from "./generated/contract";
import { usd } from "./useFor";

/** A budget's two limits as settings hold them: null is no limit. */
export type Limits = { dayUsd: number | null; monthUsd: number | null };

export function limitsOf(budget: SpendingBudget): Limits {
	return { dayUsd: budget.dayUsd, monthUsd: budget.monthUsd };
}

/** A limit as the row says it: none set is no limit, not zero. */
export function limitText(budget: SpendingBudget): string {
	if (budget.dayUsd === null && budget.monthUsd === null) return "No limit";
	return [budget.dayUsd === null ? undefined : `${usd(budget.dayUsd)} a day`, budget.monthUsd === null ? undefined : `${usd(budget.monthUsd)} a month`]
		.filter(Boolean)
		.join(", ");
}

/** The whole `spending` setting with one budget's limits changed, so a write keeps the others. */
export function budgetsPatch(spending: CapabilitySpending, kind: BudgetKind, next: Limits): Record<string, Limits> {
	const patch: Record<string, Limits> = {};
	for (const budget of spending.budgets) patch[budget.kind] = budget.kind === kind ? next : limitsOf(budget);
	return patch;
}
