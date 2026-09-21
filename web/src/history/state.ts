import type { HistoricalEvent } from "./types";

export const MAX_HISTORY_ROWS = 1_000;

export function mergeHistoryEvents(
  current: HistoricalEvent[],
  incoming: HistoricalEvent[],
): HistoricalEvent[] {
  const bySequence = new Map<string, HistoricalEvent>();
  for (const event of [...current, ...incoming]) bySequence.set(event.sequence, event);
  return [...bySequence.values()]
    .sort((a, b) => compareDecimalSequence(b.sequence, a.sequence))
    .slice(0, MAX_HISTORY_ROWS);
}

export function compareDecimalSequence(left: string, right: string): number {
  const normalizedLeft = left.replace(/^0+(?=\d)/, "");
  const normalizedRight = right.replace(/^0+(?=\d)/, "");
  if (normalizedLeft.length !== normalizedRight.length) {
    return normalizedLeft.length - normalizedRight.length;
  }
  return normalizedLeft.localeCompare(normalizedRight);
}

export function metricValue(value: string): number {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : 0;
}
