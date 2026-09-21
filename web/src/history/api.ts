import type {
  HistoricalConfigRevision,
  HistoricalDrift,
  HistoricalEvent,
  HistoricalLineage,
  HistoricalMetrics,
  HistoricalSubject,
  HistoricalUpdate,
  HistoryPage,
  HistoryStatus,
} from "./types";

async function readHistoryJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let code = `history_${response.status}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) code = body.error;
    } catch {
      // Preserve the stable fallback code.
    }
    const error = new Error(code) as Error & { status?: number };
    error.status = response.status;
    throw error;
  }
  return response.json() as Promise<T>;
}

export function getHistoryStatus(signal?: AbortSignal): Promise<HistoryStatus> {
  return fetch("/api/history/v1/status", { signal, cache: "no-store" }).then(
    readHistoryJson<HistoryStatus>,
  );
}

export function getHistoryEvents(
  cursor?: string,
  signal?: AbortSignal,
): Promise<HistoryPage<HistoricalEvent>> {
  const search = new URLSearchParams({ limit: "50" });
  if (cursor) search.set("cursor", cursor);
  return fetch(`/api/history/v1/events?${search}`, { signal, cache: "no-store" }).then(
    readHistoryJson<HistoryPage<HistoricalEvent>>,
  );
}

export function getHistoryUpdates(signal?: AbortSignal): Promise<HistoryPage<HistoricalUpdate>> {
  return fetch("/api/history/v1/updates?limit=50", { signal, cache: "no-store" }).then(
    readHistoryJson<HistoryPage<HistoricalUpdate>>,
  );
}

export function getHistoryDrift(signal?: AbortSignal): Promise<HistoryPage<HistoricalDrift>> {
  return fetch("/api/history/v1/drift?limit=50", { signal, cache: "no-store" }).then(
    readHistoryJson<HistoryPage<HistoricalDrift>>,
  );
}

export function getHistoryMetrics(signal?: AbortSignal): Promise<HistoricalMetrics> {
  return fetch("/api/history/v1/metrics?resolution=total", {
    signal,
    cache: "no-store",
  }).then(readHistoryJson<HistoricalMetrics>);
}


export function getHistoryConfigRevisions(
  signal?: AbortSignal,
): Promise<HistoryPage<HistoricalConfigRevision>> {
  return fetch("/api/history/v1/config-revisions?limit=50", {
    signal,
    cache: "no-store",
  }).then(readHistoryJson<HistoryPage<HistoricalConfigRevision>>);
}

export function getHistorySubjects(
  signal?: AbortSignal,
): Promise<HistoryPage<HistoricalSubject>> {
  return fetch("/api/history/v1/subjects?limit=50&category=component", {
    signal,
    cache: "no-store",
  }).then(readHistoryJson<HistoryPage<HistoricalSubject>>);
}

export function getHistoryLineage(
  subjectId: string,
  signal?: AbortSignal,
): Promise<HistoricalLineage> {
  return fetch(`/api/history/v1/subjects/${encodeURIComponent(subjectId)}/lineage`, {
    signal,
    cache: "no-store",
  }).then(readHistoryJson<HistoricalLineage>);
}
