export function createCliClosingStateController() {
  let closing = false

  return {
    isClosing: () => closing,
    setClosing: (value: boolean) => {
      closing = value
    },
    markClosing: () => {
      closing = true
    },
  }
}
