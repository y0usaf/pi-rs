# pi-mcp

Pi's Model Context Protocol client ([@earendil-works/pi-mcp](https://github.com/earendil-works/pi/tree/main/packages/mcp)):
JSON-RPC over stdio or streamable HTTP, the protocol's result types, and a tool result's content as
the text and images a model takes, following the upstream TypeScript at bdb1517.

Synchronous: one client owns one transport, sends a request and reads messages until its answer
arrives, answering the server's own ping and roots/list on the way. It lists tools across pages and
calls them.

It won't: open the server-to-client GET stream, so the server's notifications (tools changing, log
messages, progress) do not arrive; send notifications/cancelled when a call is abandoned; or sign in
with OAuth, so an HTTP server that answers 401 fails instead. Those are the pieces
packages/mcp/src/oauth and the GET stream add upstream.
