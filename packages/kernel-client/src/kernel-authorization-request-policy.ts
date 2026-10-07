// These requests wait for a human, and sudo can also wait for an idle agent.
// The kernel owns expiry/revocation. Replaying can duplicate the authorization.
export function waitsForKernelAuthorization(request: unknown): boolean {
  if (typeof request !== "object" || request === null) return false
  if ("RequestKernelAccess" in request || "RequestKernelSudo" in request || "ExtendKernelSudo" in request) return true
  if (!("SubmitPrompt" in request)) return false
  const submission = request.SubmitPrompt
  return typeof submission === "object" && submission !== null
    && "prompt" in submission && typeof submission.prompt === "string"
    && /^\/sudo(?:\s|$)/u.test(submission.prompt.trimStart())
}
