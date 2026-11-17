import { describe, expect, test } from "bun:test";
import { parseDelimited } from "../src/delimited";

describe("a CSV or TSV file read into rows", () => {
	test("plain rows are split at the commas and the line ends", () => {
		expect(parseDelimited("Day,Sign-ups\nMon,121\nTue,158\n", ",")).toEqual([
			["Day", "Sign-ups"],
			["Mon", "121"],
			["Tue", "158"],
		]);
		expect(parseDelimited("a,b", ",")).toEqual([["a", "b"]]);
		expect(parseDelimited("", ",")).toEqual([]);
	});

	test("a quoted field keeps its commas", () => {
		expect(parseDelimited('name,city\n"Lovelace, Ada","London, UK"\n', ",")).toEqual([
			["name", "city"],
			["Lovelace, Ada", "London, UK"],
		]);
	});

	test("a quote written twice inside a quoted field is one quote, and a quote mid-field is a character", () => {
		expect(parseDelimited('said,size\n"She said ""hi""",5" screen\n', ",")).toEqual([
			["said", "size"],
			['She said "hi"', '5" screen'],
		]);
		expect(parseDelimited('a,"",b\n', ",")).toEqual([["a", "", "b"]]);
	});

	test("a quoted field keeps its line breaks", () => {
		expect(parseDelimited('note,n\n"first line\nsecond line",2\n"cr\r\nlf",3\n', ",")).toEqual([
			["note", "n"],
			["first line\nsecond line", "2"],
			["cr\r\nlf", "3"],
		]);
	});

	test("CRLF and a lone CR end a line as LF does", () => {
		expect(parseDelimited("a,b\r\n1,2\r\n3,4\r5,6", ",")).toEqual([
			["a", "b"],
			["1", "2"],
			["3", "4"],
			["5", "6"],
		]);
	});

	test("a byte order mark is not part of the first cell", () => {
		expect(parseDelimited("﻿Day,Sign-ups\r\nMon,121\r\n", ",")).toEqual([
			["Day", "Sign-ups"],
			["Mon", "121"],
		]);
		expect(parseDelimited('﻿"Day",n\n', ",")).toEqual([["Day", "n"]]);
	});

	test("ragged rows keep the length they were written with, and a blank line is no row", () => {
		expect(parseDelimited("a,b,c\n1\n\n2,3,4,5\n4,\n", ",")).toEqual([["a", "b", "c"], ["1"], ["2", "3", "4", "5"], ["4", ""]]);
	});

	test("a TSV is split at its tabs, and its commas are characters", () => {
		expect(parseDelimited('Day\tNote\nMon\tslow, then fast\nTue\t"a\tquoted tab"\n', "\t")).toEqual([
			["Day", "Note"],
			["Mon", "slow, then fast"],
			["Tue", "a\tquoted tab"],
		]);
	});
});
