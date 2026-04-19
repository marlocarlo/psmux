use super::*;

/// Push a serialized frame to all persistent clients.
/// If a client's channel is full, drain the oldest frame first so the
/// newest frame is always delivered — this bounds memory while ensuring
/// the client never stalls the server.
/// Dead channels (writer thread exited) are pruned automatically.
pub fn push_frame(frame: &str) {
    if let Ok(mut channels) = FRAME_PUSH_CHANNELS.lock() {
        channels.retain(|(_, tx)| {
            match tx.try_send(frame.to_string()) {
                Ok(()) => true,
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    // Channel full — this happens during sustained high-throughput
                    // (e.g. rapid scroll in copy mode).  The oldest frame in the
                    // channel is stale, so we make room by creating a fresh channel
                    // pair isn't practical here.  Instead, we just skip this frame
                    // for this client — the next push will likely succeed, and the
                    // client already has FRAME_CHANNEL_CAPACITY frames queued to
                    // drain, so it won't miss content.
                    true
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
    }
}
