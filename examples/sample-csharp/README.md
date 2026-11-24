# sample-csharp

A tiny C# domain (an `Order` with `Submit`, an `OrderService`, a couple of tests)
used to demo Concede's decision log end to end. It is not built in CI; it only
needs to be realistic enough to edit in a Claude Code session and then query with
`concede why src/Order.cs` and `concede inbox`.

Use `examples/try-it.sh` (or `try-it.ps1`) to copy this into a scratch repo wired
up for Claude Code.
