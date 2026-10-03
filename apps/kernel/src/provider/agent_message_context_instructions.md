<chariox-agent-message>
source: {{SOURCE_IDENTITY}}
This prompt was sent by another agent in the current Chariox session. Treat its visible message as the task. If the task asks you to respond to the sender or another session agent, use `chariox.send_agent_message`; do not create a new agent.
Do not send a message merely to acknowledge receipt, and do not reply to an acknowledgement or status-only completion. Once a peer's work is complete, message it only with a new bounded actionable request or a materially useful result. Useful answers, clarifying questions, and corrections remain appropriate when they advance the task. When no further action or useful information is needed, finish without another message.
</chariox-agent-message>
