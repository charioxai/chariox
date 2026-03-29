import { ARROBA_ASCII_ART, type SessionListEntry } from "./sessions.js"
import {
  catalogModelOptions,
  selectConfiguredModel,
  selectConfiguredVariant,
  type CatalogModelOption,
  type ProviderCatalog,
} from "./provider-catalog.js"

export type WaitingRoomFocus = "new" | "join" | "model" | "effort"

export type WaitingRoomKeyState = {
  up: boolean
  down: boolean
  left: boolean
  right: boolean
}

export type WaitingRoomState = {
  focus: WaitingRoomFocus
  sessionIndex: number
  modelId: string
  effort: string
  introStep: number
  keyState: WaitingRoomKeyState
}

export function createWaitingRoomState(
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  model: string,
  effort: string,
): WaitingRoomState {
  const selected = selectConfiguredModel(catalog, model)
  return normalizeWaitingRoomState(
    {
      focus: "new",
      sessionIndex: 0,
      modelId: selected?.id ?? model,
      effort: selectConfiguredVariant(selected, effort),
      introStep: 0,
      keyState: { up: false, down: false, left: false, right: false },
    },
    sessions,
    catalog,
  )
}

export function normalizeWaitingRoomState(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
) {
  const options = catalogModelOptions(catalog)
  const selected = options.find((option) => option.id === state.modelId) ?? options[0] ?? null
  const efforts = waitingRoomEfforts(selected)
  return {
    ...state,
    sessionIndex: sessions.length === 0 ? 0 : modulo(state.sessionIndex, sessions.length),
    modelId: selected?.id ?? state.modelId,
    effort: efforts.includes(state.effort) ? state.effort : efforts[0] ?? "",
  }
}

export function waitingRoomModel(state: WaitingRoomState, catalog: ProviderCatalog) {
  return catalogModelOptions(catalog).find((option) => option.id === state.modelId) ?? null
}

export function waitingRoomEfforts(option: CatalogModelOption | null) {
  if (!option || option.variants.length === 0) {
    return [""]
  }
  return option.variants
}

export function moveWaitingRoomFocus(state: WaitingRoomState, delta: number) {
  const order: WaitingRoomFocus[] = ["new", "join", "model", "effort"]
  return {
    ...state,
    focus: order[modulo(order.indexOf(state.focus) + delta, order.length)]!,
  }
}

export function cycleWaitingRoomValue(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  delta: number,
) {
  if (state.focus === "join") {
    if (sessions.length === 0) {
      return state
    }
    return {
      ...state,
      sessionIndex: modulo(state.sessionIndex + delta, sessions.length),
    }
  }
  if (state.focus === "model") {
    const options = catalogModelOptions(catalog)
    if (options.length === 0) {
      return state
    }
    const index = Math.max(0, options.findIndex((option) => option.id === state.modelId))
    const next = options[modulo(index + delta, options.length)]!
    return normalizeWaitingRoomState(
      {
        ...state,
        modelId: next.id,
      },
      sessions,
      catalog,
    )
  }
  if (state.focus === "effort") {
    const efforts = waitingRoomEfforts(waitingRoomModel(state, catalog))
    const index = Math.max(0, efforts.indexOf(state.effort))
    return {
      ...state,
      effort: efforts[modulo(index + delta, efforts.length)] ?? "",
    }
  }
  return state
}

export function waitingRoomChoice(state: WaitingRoomState, sessions: SessionListEntry[], catalog: ProviderCatalog) {
  const model = waitingRoomModel(state, catalog)
  return {
    session: sessions[state.sessionIndex] ?? null,
    model,
    effort: state.effort,
  }
}

export function waitingRoomRows(state: WaitingRoomState, sessions: SessionListEntry[], catalog: ProviderCatalog) {
  const choice = waitingRoomChoice(state, sessions, catalog)
  return [
    {
      id: "new" as const,
      title: "Start New Session",
      value: "Press Enter",
    },
    {
      id: "join" as const,
      title: "Join Existing Session",
      value: choice.session ? `${choice.session.alias ?? choice.session.id} · ${choice.session.status.toLowerCase()}` : "No sessions available",
    },
    {
      id: "model" as const,
      title: "Model",
      value: choice.model ? `${choice.model.providerName} ${choice.model.label}` : "No models available",
    },
    {
      id: "effort" as const,
      title: "Effort",
      value: choice.effort ? formatTitleCase(choice.effort) : "Default",
    },
  ]
}

export function arrobaArtFrame(step: number) {
  const progress = Math.max(0, Math.min(step, 12))
  return ARROBA_ASCII_ART.split("\n")
    .map((line, row) =>
      [...line]
        .map((char, index) => {
          if (char === " ") {
            return " "
          }
          const threshold = Math.floor(((row * 7 + index) % 13) + progress)
          if (threshold >= 12) {
            return char
          }
          return [".", "*", "+", "#"][modulo(row + index + step, 4)]!
        })
        .join(""),
    )
    .join("\n")
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}
