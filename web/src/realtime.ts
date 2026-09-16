import type { StudioEvent } from "./types";

export type ConnectionState = "connecting" | "connected" | "reconnecting" | "disconnected";

export interface RealtimeCallbacks {
  onEvent: (event: StudioEvent) => void;
  onState: (state: ConnectionState) => void;
}

export function connectRealtime(callbacks: RealtimeCallbacks): () => void {
  let socket: WebSocket | null = null;
  let stopped = false;
  let retryMs = 500;
  let reconnectTimer: number | null = null;

  const connect = () => {
    if (stopped) return;
    callbacks.onState(socket ? "reconnecting" : "connecting");
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(`${protocol}//${window.location.host}/api/ws`);

    socket.onopen = () => {
      retryMs = 500;
      callbacks.onState("connected");
    };

    socket.onmessage = (message) => {
      try {
        callbacks.onEvent(JSON.parse(message.data) as StudioEvent);
      } catch {
        // Ignore malformed frames; the next snapshot/event can recover state.
      }
    };

    socket.onclose = () => {
      socket = null;
      if (stopped) {
        callbacks.onState("disconnected");
        return;
      }
      callbacks.onState("reconnecting");
      reconnectTimer = window.setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 10_000);
    };

    socket.onerror = () => socket?.close();
  };

  connect();

  return () => {
    stopped = true;
    if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
    socket?.close();
    callbacks.onState("disconnected");
  };
}
