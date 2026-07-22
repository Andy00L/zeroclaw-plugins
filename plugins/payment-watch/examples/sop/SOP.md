# payment-watch

Poll the open invoice's reference address and report settlement. The agent
keeps the reference, expected amount, and latest cursor in the conversation
(the tool is stateless by design; the cursor rides in its output).

## Steps

1. **Check settlement** — Call the settlement check for the currently open
   invoice reference with the expected amount; include the cursor from the
   previous poll when one exists.
   - tools: payment_watch
   - allow-tools: payment_watch

2. **Announce when paid** — Only when step 1 reports PAID or PARTIAL,
   message the operator's channel with the status line and the on-chain
   evidence; stay silent on PENDING.
   - depends_on: 1
