// MP-08 / MP-10: an expected bootstrap outage must reach the real product UI.
// Successful malformed or wrong-target responses still fail the drill.
export function observeRoomFaultBootstrap(body, metadata, validate) {
  if (!Number.isInteger(metadata?.httpStatus)) throw new Error('bootstrap HTTP status missing')
  if (metadata.httpStatus < 200 || metadata.httpStatus >= 300) return false
  validate(body)
  return true
}
