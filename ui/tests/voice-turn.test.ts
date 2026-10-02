import { describe, expect, test } from "bun:test";
import {
  IDLE_DROP_MS,
  isGoodbye,
  levelFromDb,
  MAX_TURN_MS,
  MIN_SPEECH_MS,
  SILENCE_MS,
  TurnDetector,
  type TurnEvent,
} from '../src/voice/turn';

const SPEECH = 0.1;
const QUIET = 0.004;

/**
 * A person talking from `from` to `to`: loud and softer by turns, the way
 * syllables are. A phone's smoothed meter can also hold nearly still during
 * speech, which must not raise the background floor once it is confirmed.
 */
function talk(detector: TurnDetector, from: number, to: number, every = 16): TurnEvent[] {
  const events: TurnEvent[] = [];
  for (let now = from; now < to; now += every) {
    const syllable = Math.floor(now / 120) % 2 === 0;
    events.push(...detector.push(syllable ? 0.12 : 0.05, now));
  }
  return events;
}

/** A fan at `level` from `from` to `to`: steady, give or take a few percent. */
function fan(
  detector: TurnDetector,
  level: number,
  from: number,
  to: number,
  every = 16,
): TurnEvent[] {
  const events: TurnEvent[] = [];
  for (let now = from; now < to; now += every) {
    events.push(...detector.push(level * (1 + 0.1 * Math.sin(now / 90)), now));
  }
  return events;
}

/** What happened to a turn, without the clips of silence dropped every 8 seconds on the way. */
const turns = (events: TurnEvent[]) =>
  events.filter((event) => !(event.kind === 'drop' && event.reason === 'idle'));

/** A meter reporting `level` from `from` to `to`, every `every` ms. */
function run(
  detector: TurnDetector,
  level: number,
  from: number,
  to: number,
  every = 16,
): TurnEvent[] {
  const events: TurnEvent[] = [];
  for (let now = from; now < to; now += every) events.push(...detector.push(level, now));
  return events;
}

describe('a turn', () => {
  test('starts once the voice outlasts a blip, and ends after 1.2s of quiet', () => {
    const detector = new TurnDetector(0);
    const talking = run(detector, SPEECH, 0, 1500);
    expect(talking.map((event) => event.kind)).toEqual(['start']);
    expect(talking[0]).toEqual({ kind: 'start', at: 0 });

    const after = run(detector, QUIET, 1500, 3000);
    expect(after).toHaveLength(1);
    const end = after[0]!;
    expect(end.kind).toBe('end');
    expect(end.kind === 'end' && end.reason).toBe('silence');
    // The last voiced sample was at 1488, so the end lands just past 1.2s after it.
    expect(end.at).toBeGreaterThan(1488 + SILENCE_MS);
    expect(end.at).toBeLessThanOrEqual(1488 + SILENCE_MS + 32);
  });

  test('ignores a blip shorter than 250ms, and the clip carries on', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, QUIET, 0, 1000),
      ...run(detector, SPEECH, 1000, 1000 + MIN_SPEECH_MS - 16),
      ...run(detector, QUIET, 1000 + MIN_SPEECH_MS - 16, 4000),
    ];
    expect(events).toEqual([]);
  });

  test('a blip in a silent clip does not keep the clip from being dropped', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, SPEECH, 1000, 1100),
      ...run(detector, QUIET, 1100, IDLE_DROP_MS + 2000),
    ];
    expect(events.map((event) => event.kind)).toEqual(['drop']);
  });

  test('a pause just shorter than 1.2s does not split a sentence', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, SPEECH, 0, 800),
      ...run(detector, QUIET, 800, 800 + SILENCE_MS - 100),
      ...run(detector, SPEECH, 800 + SILENCE_MS - 100, 2500),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start']);
  });

  test('is cut at 20 seconds however long the person keeps talking', () => {
    const detector = new TurnDetector(0);
    const events = talk(detector, 0, MAX_TURN_MS + 1000);
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    const end = events[1]!;
    expect(end.kind === 'end' && end.reason).toBe('max');
    expect(end.at).toBeGreaterThan(MAX_TURN_MS);
    expect(end.at).toBeLessThanOrEqual(MAX_TURN_MS + 16);
  });

  test('counts the 20 seconds from the start of the clip, silence included', () => {
    const detector = new TurnDetector(0);
    const events = [...run(detector, QUIET, 0, 7000), ...talk(detector, 7000, MAX_TURN_MS + 1000)];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(events[1]!.at).toBeLessThanOrEqual(MAX_TURN_MS + 16);
  });

  test('ignores everything after it ends, until the client resets it', () => {
    const detector = new TurnDetector(0);
    run(detector, SPEECH, 0, 1000);
    run(detector, QUIET, 1000, 2500);
    expect(run(detector, SPEECH, 2500, 3500)).toEqual([]);

    detector.reset(3500);
    const again = run(detector, SPEECH, 3500, 4500);
    expect(again.map((event) => event.kind)).toEqual(['start']);
    expect(again[0]).toEqual({ kind: 'start', at: 3500 });
  });

  test('starts from a clock that reads zero', () => {
    const detector = new TurnDetector(0);
    expect(run(detector, SPEECH, 0, 400)[0]).toEqual({ kind: 'start', at: 0 });
  });

  test('separate 50ms beeps cannot borrow the silence between them to start speech', () => {
    const detector = new TurnDetector(0);
    const events: TurnEvent[] = [];
    for (let now = 0; now < 3000; now += 50)
      events.push(...detector.push(now % 400 === 0 ? SPEECH : QUIET, now));
    expect(events).toEqual([]);
  });

  test('sparse 100ms alerts never accumulate across onset windows', () => {
    const detector = new TurnDetector(0);
    const events: TurnEvent[] = [];
    for (let now = 0; now < 3000; now += 50)
      events.push(...detector.push(now % 400 < 100 ? SPEECH : QUIET, now));
    expect(events).toEqual([]);
  });

  test('brief syllable gaps are allowed but do not count as voiced duration', () => {
    const detector = new TurnDetector(0);
    const events: TurnEvent[] = [];
    for (let now = 0; now <= 450; now += 50) {
      events.push(...detector.push(now === 200 ? QUIET : SPEECH, now));
      // At 300ms, 50ms belongs to the quiet gap and 250ms to voiced audio.
      if (now < 300) expect(events).toEqual([]);
    }
    expect(events.map((event) => event.kind)).toEqual(['start']);
  });

  test('100ms syllables with quiet gaps can establish an utterance', () => {
    for (const period of [150, 200]) {
      const detector = new TurnDetector(0);
      const events: TurnEvent[] = [];
      for (let now = 0; now < 2000; now += 50)
        events.push(...detector.push(now % period < 100 ? SPEECH : QUIET, now));
      expect(events.map((event) => event.kind)).toEqual(['start']);
    }
  });

  test('a 900ms conversational pause leaves time for the next words', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, SPEECH, 0, 700, 50),
      ...run(detector, QUIET, 700, 1600, 50),
      ...run(detector, SPEECH, 1600, 2300, 50),
      ...run(detector, QUIET, 2300, 3800, 50),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(events[1]!.at).toBeGreaterThan(2250 + SILENCE_MS);
  });

  test('quiet trailing words keep a confirmed utterance alive without starting a new one', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, SPEECH, 0, 600, 50),
      ...run(detector, 0.015, 600, 3000, 50),
      ...run(detector, QUIET, 3000, 4500, 50),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(events[1]!.at).toBeGreaterThan(2950 + SILENCE_MS);

    const fresh = new TurnDetector(0);
    expect(run(fresh, 0.015, 0, 3000, 50)).toEqual([]);
  });

  test('quiet trailing speech over a learned room uses the continuation margin', () => {
    const detector = new TurnDetector(0);
    run(detector, 0.014, 0, 6000, 50);
    detector.reset(6000);
    const events = [
      ...run(detector, SPEECH, 6000, 6600, 50),
      ...run(detector, 0.03, 6600, 9000, 50),
      ...run(detector, 0.014, 9000, 10_500, 50),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(events[1]!.at).toBeGreaterThan(8950 + SILENCE_MS);
  });
});

describe('unavailable meter readings', () => {
  test('missing or invalid audio never lowers the noise floor', () => {
    const detector = new TurnDetector(0);
    const floor = detector.noiseFloor;
    for (const [at, level] of [
      [50, null],
      [100, NaN],
      [150, Infinity],
      [200, -1],
    ] as const)
      expect(detector.push(level, at)).toEqual([]);
    expect(detector.noiseFloor).toBe(floor);
  });

  test('missing readings do not turn an isolated spike into an onset', () => {
    const detector = new TurnDetector(0);
    detector.push(SPEECH, 0);
    for (let now = 50; now <= 400; now += 50) detector.push(null, now);
    expect(detector.push(SPEECH, 450)).toEqual([]);
    expect(detector.push(QUIET, 500)).toEqual([]);
  });

  test('missing readings pause the endpoint clock until silence is measured again', () => {
    const detector = new TurnDetector(0);
    run(detector, SPEECH, 0, 600, 50);
    const floor = detector.noiseFloor;
    for (let now = 600; now <= 2500; now += 50) expect(detector.push(null, now)).toEqual([]);
    expect(detector.noiseFloor).toBe(floor);
    const after = run(detector, QUIET, 2550, 4000, 50);
    expect(after.map((event) => event.kind)).toEqual(['end']);
    expect(after[0]!.at).toBeGreaterThan(2500 + SILENCE_MS);
  });

  test('a long callback gap does not count as a measured pause or sustained voice', () => {
    const detector = new TurnDetector(0);
    expect(detector.push(SPEECH, 0)).toEqual([]);
    expect(detector.push(SPEECH, 1000)).toEqual([]);
    expect(run(detector, SPEECH, 1050, 1500, 50).map((event) => event.kind)).toEqual(['start']);
    expect(detector.push(QUIET, 3500)).toEqual([]);
    expect(run(detector, QUIET, 3550, 4200, 50)).toEqual([]);
    expect(run(detector, QUIET, 4200, 5000, 50).map((event) => event.kind)).toEqual(['end']);
  });

  test('regular 100ms and jittery callbacks can still establish speech', () => {
    const detector = new TurnDetector(0);
    expect(run(detector, SPEECH, 0, 500, 100).map((event) => event.kind)).toEqual(['start']);
    const jittery = new TurnDetector(0);
    const events = [0, 45, 110, 150, 240, 310, 430].flatMap((at) => jittery.push(SPEECH, at));
    expect(events.map((event) => event.kind)).toEqual(['start']);
  });
});

describe('a clip of pure silence', () => {
  test('is dropped after 8 seconds and the next one starts clean', () => {
    const detector = new TurnDetector(0);
    const first = run(detector, QUIET, 0, IDLE_DROP_MS + 100);
    expect(first).toHaveLength(1);
    expect(first[0]!.kind).toBe('drop');

    // The drop began a new clip: a whole turn follows without a reset call.
    const from = IDLE_DROP_MS + 100;
    const turn = [
      ...run(detector, SPEECH, from, from + 1000),
      ...run(detector, QUIET, from + 1000, from + 2500),
    ];
    expect(turn.map((event) => event.kind)).toEqual(['start', 'end']);
  });

  test('is not dropped under a person who begins speaking as the 8 seconds run out', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, QUIET, 0, IDLE_DROP_MS - 48),
      ...run(detector, SPEECH, IDLE_DROP_MS - 48, IDLE_DROP_MS + 1500),
      ...run(detector, QUIET, IDLE_DROP_MS + 1500, IDLE_DROP_MS + 3000),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
  });
});

describe('the noise floor', () => {
  test('rises to a steady room and stops counting it as voice', () => {
    const level = 0.03;
    const fresh = new TurnDetector(0);
    expect(run(fresh, level, 0, 1000).map((event) => event.kind)).toEqual(['start']);

    // The same level, but the room has hummed at 0.014 for a while first.
    const learned = new TurnDetector(0);
    run(learned, 0.014, 0, 6000);
    expect(learned.noiseFloor).toBeCloseTo(0.014, 3);
    learned.reset(6000);
    expect(run(learned, level, 6000, 7000)).toEqual([]);
    // Speech well above the room still gets through.
    expect(run(learned, SPEECH, 7000, 8000).map((event) => event.kind)).toEqual(['start']);
  });

  test('is learned at the same pace however often the meter reports', () => {
    const fast = new TurnDetector(0);
    const slow = new TurnDetector(0);
    run(fast, 0.012, 0, 2000, 16);
    run(slow, 0.012, 0, 2000, 100);
    expect(Math.abs(fast.noiseFloor - slow.noiseFloor)).toBeLessThan(0.0005);
  });

  test('does not learn speech as the room', () => {
    const detector = new TurnDetector(0);
    talk(detector, 0, 15_000);
    expect(detector.noiseFloor).toBeCloseTo(0.008, 5);
  });

  test('does not let a long turn of speech become its own noise', () => {
    const detector = new TurnDetector(0);
    const events = [...talk(detector, 0, 15_000), ...run(detector, QUIET, 15_000, 16_500)];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    const end = events[1]!;
    expect(end.kind === 'end' && end.reason).toBe('silence');
  });

  test('survives a meter that reports nonsense', () => {
    const detector = new TurnDetector(0);
    expect(detector.push(Number.NaN, 0)).toEqual([]);
    expect(detector.push(-1, 16)).toEqual([]);
    expect(detector.push(Number.POSITIVE_INFINITY, 32)).toEqual([]);
    expect(Number.isFinite(detector.noiseFloor)).toBe(true);
  });
});

describe('a room that gets louder', () => {
  test('a steady room below the onset threshold is learned without opening a turn', () => {
    const detector = new TurnDetector(0);
    expect(run(detector, QUIET, 0, 1000)).toEqual([]);
    const events = fan(detector, 0.015, 1000, 12_000);
    expect(turns(events)).toEqual([]);
    expect(detector.noiseFloor).toBeGreaterThan(0.013);
  });

  test('the learned room carries over to each idle recording', () => {
    const detector = new TurnDetector(0);
    fan(detector, 0.02, 0, 12_000);
    expect(turns(fan(detector, 0.02, 12_000, 60_000))).toEqual([]);
    // The floor keeps closing on the fan's own level, and stays under it.
    expect(detector.noiseFloor).toBeGreaterThan(0.018);
    expect(detector.noiseFloor).toBeLessThan(0.022);
  });

  test('a person talking over a fan that has been learned is still heard', () => {
    const detector = new TurnDetector(0);
    fan(detector, 0.02, 0, 20_000);
    const events: TurnEvent[] = [];
    for (let now = 20_000; now < 22_000; now += 16) {
      const syllable = Math.floor(now / 120) % 2 === 0;
      events.push(...detector.push((syllable ? 0.3 : 0.15) + 0.02, now));
    }
    events.push(...fan(detector, 0.02, 22_000, 24_000));
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    const end = events[1]!;
    expect(end.kind === 'end' && end.reason).toBe('silence');
  });

  test('a smoothed steady voice is not reclassified as noise after onset', () => {
    const detector = new TurnDetector(0);
    const events = [
      ...run(detector, SPEECH, 0, 5000),
      ...run(detector, 0.015, 5000, 8000),
      ...run(detector, QUIET, 8000, 9500),
    ];
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(detector.noiseFloor).toBeCloseTo(0.008, 5);
    expect(events[1]!.at).toBeGreaterThan(7984 + SILENCE_MS);
  });

  test('a long loud tone remains ambiguous to a meter and is bounded by the clip limit', () => {
    // Energy-only detection cannot reject this without also rejecting steady speech.
    const detector = new TurnDetector(0);
    const events = run(detector, SPEECH, 0, MAX_TURN_MS + 1000);
    expect(events.map((event) => event.kind)).toEqual(['start', 'end']);
    expect(events[1]!.kind === 'end' && events[1]!.reason).toBe('max');
  });

  test('a level that is only a little above the floor, never loud enough to start a turn, still raises it', () => {
    const detector = new TurnDetector(0);
    // 0.02 is over twice the floor (0.008) and under what starts a turn (0.0256).
    expect(turns(run(detector, 0.02, 0, 20_000))).toEqual([]);
    expect(detector.noiseFloor).toBeGreaterThan(0.017);
  });

  test('the floor rises at the same pace however often the meter reports', () => {
    const fast = new TurnDetector(0);
    const slow = new TurnDetector(0);
    run(fast, 0.02, 0, 10_000, 16);
    run(slow, 0.02, 0, 10_000, 100);
    expect(Math.abs(fast.noiseFloor - slow.noiseFloor)).toBeLessThan(0.002);
  });

  test('silence after a noisy room lets the floor fall again', () => {
    const detector = new TurnDetector(0);
    fan(detector, 0.02, 0, 15_000);
    const loud = detector.noiseFloor;
    run(detector, QUIET, 15_000, 25_000);
    expect(detector.noiseFloor).toBeLessThan(loud / 2);
  });
});

describe('levels from decibels', () => {
  test('full scale is 1, minus 20 is a tenth, and silence or junk is 0', () => {
    expect(levelFromDb(0)).toBe(1);
    expect(levelFromDb(-20)).toBeCloseTo(0.1, 6);
    expect(levelFromDb(-160)).toBeLessThan(1e-7);
    expect(levelFromDb(6)).toBe(1);
    expect(levelFromDb(Number.NEGATIVE_INFINITY)).toBe(0);
    expect(levelFromDb(Number.NaN)).toBe(0);
  });
});

describe('goodbye', () => {
  test('hears a farewell that is the whole utterance', () => {
    for (const said of [
      'Bye',
      'OK, bye!',
      'Okay bye bye',
      'Thanks, goodbye.',
      'Alright, thank you, see you later',
      'See ya later, Hotline',
      'bye-bye desk',
    ]) {
      expect(isGoodbye(said)).toBe(true);
    }
  });

  test('does not hear one inside a request', () => {
    for (const said of [
      'Say goodbye to Mack',
      'Tell Mack bye and check the PR',
      'Bye the way, where is Clem',
      'See you later today Mack should look at it',
      '',
      'Hello',
    ]) {
      expect(isGoodbye(said)).toBe(false);
    }
  });
});
