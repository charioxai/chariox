import type { CommandCenterItem } from "./command-center-types.js"

export function filterCommandCenterItems(items: CommandCenterItem[], query: string) {
  if (!query) {
    return items
  }
  return items
    .map((item) => ({ item, score: scoreCommandCenterItem(item, query) }))
    .filter((entry) => entry.score > 0)
    .sort((left, right) => right.score - left.score || left.item.label.localeCompare(right.item.label))
    .map((entry) => entry.item)
    .slice(0, 20)
}

function scoreCommandCenterItem(item: CommandCenterItem, query: string) {
  const haystacks = [item.label.toLowerCase(), item.description.toLowerCase(), item.value.toLowerCase()]
  let score = 0
  for (const haystack of haystacks) {
    if (haystack.startsWith(query) || haystack.startsWith(`/${query}`)) {
      score = Math.max(score, 4)
    } else if (haystack.includes(query)) {
      score = Math.max(score, 2)
    }
  }
  for (const alias of item.searchAliases ?? []) {
    const haystack = alias.toLowerCase()
    if (haystack.startsWith(query) || haystack.startsWith(`/${query}`)) {
      score = Math.max(score, 3)
    } else if (haystack.includes(query)) {
      score = Math.max(score, 1)
    }
  }
  return score
}
