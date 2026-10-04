# Live capture and equivalent-behavior validation

The next milestone brings the current-season read gate forward. Deliver a local
capture workflow for league `394172912`, team `1`, plus offline Rust/Python
comparison of the same responses. Live access remains a separate result from
semantic equivalence. Keep the reference unchanged and captures out of Git.

The capture runs the public Rust client through a loopback recorder. Only the
recorder sends requests to the fixed ESPN read host. It records ordered request
paths, queries, decoded filters, status codes and exact response bytes. Cookies
come from paired environment variables, never from command arguments or the
bundle. Capture timestamps and response hashes make replay deterministic.

Check one selected team's complete current roster, every returned weekly
matchup/lineup, one available-player page and a single player card. Compare
identity/order, actual/projected totals and averages, per-period availability,
eligibility, injury/ownership, weekly scores, NFL opponent/rank/kickoff and card
schedules. Raw/applied breakdowns and all football capabilities remain outside
this selected live comparison; their existing offline tests still apply.

Offline verification runs a network-closed Python oracle and the Rust pure
converters on those recorded bytes. Reference initialization uses its explicit
base load and team decoder without the eager player-directory/draft enrichment;
this mirrors the documented Rust loading difference. Validate preserved request
contracts in sequence. The sole allowed request difference is the recorded
nonzero free-agent offset extension, removed for the Python request comparison
and explicitly reported. Do not normalize other request differences away.

Exit criteria: tests establish capture/replay behavior, tamper detection,
unexpected-request rejection and distinguish access, decoder, oracle and parity
failures. Local instructions are reproducible. Current/private service support
is claimed only after an actual successful capture and equivalent replay;
credential presence alone does not establish private-league coverage. This
cloud's ESPN network block is an outstanding live gate, not a passing result.

Delivered tooling checks: 18 Python workflow tests, including full capture/oracle
round trips, successful route fallback, offset-extension reporting, malformed
responses and request drift. The original 94 Rust tests and four documentation
examples remain the module regression gates. Pin the mock-server dependency to
0.6.4 so examples and tests build on Rust 1.85; CI now checks all targets on the
minimum toolchain and runs workflow tests with explicit optional-oracle skips.
