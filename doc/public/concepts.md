<span id="流、传输与保存"></span>

# Streams, transport and persistence

Each Input instance has one stream with a UUID assigned by Core and a readable description. A configured `streams[].id` is a startup binding alias; runtime stream operations use the returned UUID. Descriptions help identify streams but do not determine routing or write permissions.

A stream has one writer. Multiple Outputs receive the same stream independently, without competing for a shared consumer cursor. Subscriptions begin at the oldest retained record, follow Core's receive order, and wait for new data at the end. An Input leaving does not delete its stream. Stopping Core discards all of its memory.

Core limits each stream by both record count and byte size, overwriting the oldest records when full. Slow Outputs do not block Input publication and cannot recover evicted data from Core. Configured budgets are not a promise of unlimited resources.

| Event | What success means |
| --- | --- |
| TCP publish | Core accepted the record into memory; Outputs may not have consumed it yet |
| UDP publish | The local socket accepted the datagram; Core may not have received it |
| Output receives a record | Persistence is not yet confirmed |
| output-file finishes shutdown | The plugin drained its accepted queue and checked commit/flush results; inspect the returned status |

TCP is the default transport; UDP is optional. UDP registration has an application-level reply, but records have no per-record acknowledgment or retransmission. There is no extra TCP control channel for UDP plugins. Oversized encoded messages are rejected, not fragmented. The local CLI management port is a separate control endpoint and does not carry log traffic.

A transformation plugin reads a source stream and publishes to a separate derived stream, preserving the original. Source-sequence reordering belongs to the transformation plugin; Core does not wait for missing sequence numbers or reorder records.
