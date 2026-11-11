// Enrolling a person's own machine with a single-use Cloud ticket, local protocol 971.
export const ownMachineEnrollmentMinimumProtocolVersion = 971
export function issueOwnMachineEnrollmentRequest() {
  return { IssueOwnMachineEnrollment: {} }
}
