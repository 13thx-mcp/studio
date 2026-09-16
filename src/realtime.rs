use tokio::sync::broadcast;

use crate::{
    supervisor::{LogEntry, ProcessStatus},
    tunnel::{TunnelLogEntry, TunnelStatus},
};

pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StudioEvent {
    Snapshot {
        servers: Vec<ProcessStatus>,
        tunnel: TunnelStatus,
    },
    ProcessStatus {
        mcp_id: String,
        status: ProcessStatus,
    },
    Log {
        mcp_id: String,
        entry: LogEntry,
    },
    TunnelStatus {
        status: TunnelStatus,
    },
    TunnelLog {
        entry: TunnelLogEntry,
    },
    RegistryChanged,
    DiscoveryChanged,
    ResyncRequired,
}

#[derive(Debug, Clone)]
pub struct EventHub {
    sender: broadcast::Sender<StudioEvent>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new(EVENT_CHANNEL_CAPACITY)
    }
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StudioEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: StudioEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publishes_events_to_subscribers() {
        let hub = EventHub::new(4);
        let mut receiver = hub.subscribe();
        hub.publish(StudioEvent::ResyncRequired);

        assert!(matches!(
            receiver.recv().await.unwrap(),
            StudioEvent::ResyncRequired
        ));
    }
}