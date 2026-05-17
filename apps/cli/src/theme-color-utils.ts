export function optionalHex(value: unknown) {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value.trim())
    ? value.trim().toLowerCase()
    : null
}

export function hex(value: unknown, field: string) {
  const color = optionalHex(value)
  if (!color) {
    throw new Error(`${field} must be a #rrggbb color`)
  }
  return color
}

export function mixHex(a: string, b: string, amount: number) {
  const left = parseRgb(a)
  const right = parseRgb(b)
  const mixed = left.map((channel, index) => Math.round(channel + (right[index]! - channel) * amount))
  return `#${mixed.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`
}

export function isDarkHex(value: string) {
  const [red, green, blue] = parseRgb(value)
  return (red * 0.299 + green * 0.587 + blue * 0.114) < 128
}

export function ansiToHex(code: number) {
  if (!Number.isInteger(code) || code < 0 || code > 255) {
    return "#000000"
  }
  if (code < 16) {
    return [
      "#000000",
      "#800000",
      "#008000",
      "#808000",
      "#000080",
      "#800080",
      "#008080",
      "#c0c0c0",
      "#808080",
      "#ff0000",
      "#00ff00",
      "#ffff00",
      "#0000ff",
      "#ff00ff",
      "#00ffff",
      "#ffffff",
    ][code] ?? "#000000"
  }
  if (code < 232) {
    const index = code - 16
    const blue = index % 6
    const green = Math.floor(index / 6) % 6
    const red = Math.floor(index / 36)
    const value = (channel: number) => channel === 0 ? 0 : channel * 40 + 55
    return rgbToHex(value(red), value(green), value(blue))
  }
  const gray = (code - 232) * 10 + 8
  return rgbToHex(gray, gray, gray)
}

function rgbToHex(red: number, green: number, blue: number) {
  return `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`
}

function parseRgb(value: string): [number, number, number] {
  const normalized = value.replace("#", "")
  return [
    Number.parseInt(normalized.slice(0, 2), 16),
    Number.parseInt(normalized.slice(2, 4), 16),
    Number.parseInt(normalized.slice(4, 6), 16),
  ]
}
