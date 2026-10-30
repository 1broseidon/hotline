import { expect, test } from "bun:test";
import { excerptText } from "../src/excerpt";

test("a search result reads as words, not the markdown its bubble renders", () => {
	expect(excerptText("| Harness | Median time |\n| :--- | :--- |\n| Pi Agent | 156.2s |")).toBe("Harness · Median time Pi Agent · 156.2s");
	expect(excerptText("from [Composio's 8-harness benchmark](https://composio.dev/x): **Pi Agent** is `fastest`")).toBe("from Composio's 8-harness benchmark: Pi Agent is fastest");
	expect(excerptText("## Next\n- one *thing*\n> quoted")).toBe("Next one thing quoted");
	expect(excerptText("snake_case_name stays")).toBe("snake_case_name stays");
});
