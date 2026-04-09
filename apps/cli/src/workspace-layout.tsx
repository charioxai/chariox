import type { KeyBinding, RGBA } from "@opentui/core"
import { For } from "solid-js"

import { PromptBorderChars, SplitBorder, theme } from "./theme.js"

type RefHandler = (value: any) => void
type IndexedRefHandler = (index: number, value: any) => void

type WorkspaceLayoutProps = {
  width: number
  height: number
  fatalError: boolean
  responsePaneRows: () => number[][]
  promptPlaceholder: string
  promptInputMaxHeight: number
  promptKeyBindings: KeyBinding[]
  promptAreaBackground: RGBA
  onRootMouseUp: () => void
  onResponseSurfaceMouseUp: (event: any) => void
  onResponseLayoutBoxRef: RefHandler
  onResponseRowBoxRef: IndexedRefHandler
  onResponsePrimaryPaneRef: RefHandler
  onHistoryLoadingBoxRef: RefHandler
  onTranscriptScrollboxRef: RefHandler
  onResponsePrimaryFooterBoxRef: RefHandler
  onResponseAuxiliaryPaneRef: IndexedRefHandler
  onResponseAuxiliaryScrollboxRef: IndexedRefHandler
  onResponseAuxiliaryFooterBoxRef: IndexedRefHandler
  onCommandCenterBoxRef: RefHandler
  onPromptInputRef: RefHandler
  onPromptKeyDown: (event: any) => void
  onPromptContentChange: () => void
  onPromptSubmit: () => void
  onPromptMetaProviderTextRef: RefHandler
  onPromptMetaProviderDividerTextRef: RefHandler
  onPromptMetaModelTextRef: RefHandler
  onPromptMetaModelDividerTextRef: RefHandler
  onPromptMetaVariantTextRef: RefHandler
  onPromptMetaUsageDividerTextRef: RefHandler
  onPromptMetaUsageTokensTextRef: RefHandler
  onPromptMetaUsageBarOpenTextRef: RefHandler
  onPromptMetaUsageBarFilledTextRef: RefHandler
  onPromptMetaUsageBarEmptyTextRef: RefHandler
  onPromptMetaUsageBarCloseTextRef: RefHandler
  onPromptMetaUsagePercentTextRef: RefHandler
  onStatusIndicatorBoxRef: RefHandler
  onFooterSummaryBoxRef: RefHandler
  onHotkeysOverlayBoxRef: RefHandler
}

export function WorkspaceLayout(props: WorkspaceLayoutProps) {
  return (
    <box
      width={props.width}
      height={props.height}
      flexDirection="column"
      paddingBottom={1}
      paddingLeft={2}
      paddingRight={2}
      backgroundColor={theme.background}
      onMouseUp={props.onRootMouseUp}
    >
      <box
        flexGrow={1}
        backgroundColor={theme.backgroundPanel}
        border={["left", "right"]}
        customBorderChars={SplitBorder.customBorderChars}
        borderColor={theme.borderSubtle}
        onMouseUp={props.onResponseSurfaceMouseUp}
      >
        <box
          ref={props.onResponseLayoutBoxRef}
          flexGrow={1}
          flexDirection="column"
          gap={0}
          paddingLeft={1}
          paddingRight={1}
          paddingTop={1}
          paddingBottom={1}
        >
          <For each={props.responsePaneRows()}>
            {(rowSlots, rowIndex) => (
              <box
                ref={(value) => {
                  props.onResponseRowBoxRef(rowIndex(), value)
                }}
                flexGrow={1}
                flexDirection="row"
                gap={0}
              >
                <For each={rowSlots}>
                  {(paneIndex) => (
                    paneIndex === 0
                      ? (
                          <box
                            ref={props.onResponsePrimaryPaneRef}
                            flexGrow={1}
                            flexDirection="column"
                            border={["left"]}
                            borderColor={theme.borderSubtle}
                            backgroundColor={theme.backgroundPanel}
                          >
                            <box
                              ref={props.onHistoryLoadingBoxRef}
                              flexShrink={0}
                              paddingLeft={1}
                              paddingRight={1}
                            />
                            <scrollbox
                              ref={props.onTranscriptScrollboxRef}
                              flexGrow={1}
                              stickyScroll={true}
                              stickyStart="bottom"
                              paddingLeft={2}
                              paddingRight={1}
                              paddingTop={1}
                              paddingBottom={1}
                              viewportOptions={{
                                paddingRight: 1,
                              }}
                              verticalScrollbarOptions={{
                                visible: true,
                                paddingLeft: 1,
                                trackOptions: {
                                  backgroundColor: theme.backgroundElement,
                                  foregroundColor: theme.border,
                                },
                              }}
                            />
                            <box
                              ref={props.onResponsePrimaryFooterBoxRef}
                              flexShrink={0}
                              flexDirection="row"
                              gap={1}
                              paddingLeft={1}
                              paddingRight={1}
                            />
                          </box>
                        )
                      : (
                          <box
                            ref={(value) => {
                              props.onResponseAuxiliaryPaneRef(paneIndex - 1, value)
                            }}
                            width={0}
                            flexShrink={0}
                            flexDirection="column"
                            border={false}
                            borderColor={theme.borderSubtle}
                            backgroundColor={theme.backgroundElement}
                            paddingLeft={0}
                            paddingRight={0}
                            paddingTop={0}
                            paddingBottom={0}
                            visible={false}
                          >
                            <scrollbox
                              ref={(value) => {
                                props.onResponseAuxiliaryScrollboxRef(paneIndex - 1, value)
                              }}
                              flexGrow={1}
                              stickyScroll={true}
                              stickyStart="bottom"
                              paddingLeft={2}
                              paddingRight={1}
                              paddingTop={1}
                              paddingBottom={1}
                              viewportOptions={{
                                paddingRight: 1,
                              }}
                              verticalScrollbarOptions={{
                                visible: true,
                                paddingLeft: 1,
                                trackOptions: {
                                  backgroundColor: theme.backgroundElement,
                                  foregroundColor: theme.border,
                                },
                              }}
                            />
                            <box
                              ref={(value) => {
                                props.onResponseAuxiliaryFooterBoxRef(paneIndex - 1, value)
                              }}
                              flexShrink={0}
                              flexDirection="row"
                              gap={1}
                              paddingLeft={1}
                              paddingRight={1}
                            />
                          </box>
                        )
                  )}
                </For>
              </box>
            )}
          </For>
        </box>
      </box>

      <box
        flexShrink={0}
        marginTop={1}
        overflow="visible"
        border={["left"]}
        borderColor={props.fatalError ? theme.error : theme.primary}
        customBorderChars={PromptBorderChars}
      >
        <box
          ref={props.onCommandCenterBoxRef}
          position="absolute"
          left={0}
          right={0}
          flexDirection="column"
          overflow="visible"
        />
        <box
          overflow="hidden"
          paddingLeft={2}
          paddingRight={2}
          paddingTop={1}
          paddingBottom={1}
          backgroundColor={props.promptAreaBackground}
          flexDirection="column"
          gap={1}
        >
          <box overflow="hidden">
            <textarea
              ref={props.onPromptInputRef}
              placeholder={props.promptPlaceholder}
              textColor={theme.text}
              focusedTextColor={theme.text}
              minHeight={1}
              maxHeight={props.promptInputMaxHeight}
              keyBindings={props.promptKeyBindings}
              onKeyDown={props.onPromptKeyDown}
              onContentChange={props.onPromptContentChange}
              onSubmit={props.onPromptSubmit}
            />
          </box>
          <box flexDirection="row" overflow="hidden">
            <text ref={props.onPromptMetaProviderTextRef} fg={theme.textMuted}>{" "}</text>
            <text ref={props.onPromptMetaProviderDividerTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaModelTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaModelDividerTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaVariantTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageDividerTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageTokensTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarOpenTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarFilledTextRef} fg={theme.primary}>{""}</text>
            <text ref={props.onPromptMetaUsageBarEmptyTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarCloseTextRef} fg={theme.textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsagePercentTextRef} fg={theme.textMuted}>{""}</text>
          </box>
        </box>
      </box>

      <box flexShrink={0} marginTop={1} paddingLeft={2} paddingRight={2}>
        <box flexDirection="row" gap={1}>
          <box ref={props.onStatusIndicatorBoxRef} flexDirection="row" />
          <box ref={props.onFooterSummaryBoxRef} flexDirection="row" />
        </box>
      </box>

      <box
        ref={props.onHotkeysOverlayBoxRef}
        position="absolute"
        left={0}
        top={0}
      />
    </box>
  )
}
