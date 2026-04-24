import type { KeyBinding, RGBA } from "@opentui/core"
import { For } from "solid-js"

import { PaneGridBorderChars, theme } from "./theme.js"

type RefHandler = (value: any) => void
type IndexedRefHandler = (index: number, value: any) => void

type WorkspaceLayoutProps = {
  width: number
  height: number
  fatalError: boolean
  themeRevision: number
  responsePaneRows: () => number[][]
  promptPlaceholder: string
  promptInputMaxHeight: number
  promptKeyBindings: KeyBinding[]
  promptAreaBackground: RGBA
  onRootMouseUp: () => void
  onResponseSurfaceMouseUp: (event: any) => void
  onFooterMouseUp: (event: any) => void
  onResponseLayoutBoxRef: RefHandler
  onResponseRowBoxRef: IndexedRefHandler
  onPaneGridBorderRowRef: IndexedRefHandler
  onPaneGridBottomBorderRowRef: RefHandler
  onPaneGridHorizontalSegmentRef: (rowIndex: number, segmentIndex: number, value: any) => void
  onPaneGridBottomHorizontalSegmentRef: (segmentIndex: number, value: any) => void
  onPaneGridJunctionTextRef: (rowIndex: number, junctionIndex: number, value: any) => void
  onPaneGridBottomJunctionTextRef: (junctionIndex: number, value: any) => void
  onPaneGridVerticalSegmentRef: (rowIndex: number, segmentIndex: number, value: any) => void
  onResponsePrimaryPaneRef: RefHandler
  onHistoryLoadingBoxRef: RefHandler
  onTranscriptScrollboxRef: RefHandler
  onResponsePrimaryInteractionBoxRef: RefHandler
  onResponsePrimaryFooterBoxRef: RefHandler
  onResponseAuxiliaryPaneRef: IndexedRefHandler
  onResponseAuxiliaryScrollboxRef: IndexedRefHandler
  onResponseAuxiliaryInteractionBoxRef: IndexedRefHandler
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
  const palette = () => {
    props.themeRevision
    return theme
  }
  const renderPaneSlot = (paneIndex: number | undefined) => {
    if (paneIndex === undefined) {
      return null
    }
    return paneIndex === 0
      ? (
          <box
            ref={props.onResponsePrimaryPaneRef}
            flexGrow={1}
            flexBasis={0}
            flexDirection="column"
            border={false}
            borderColor={palette().borderSubtle}
            backgroundColor={palette().backgroundPanel}
          >
            <box
              ref={props.onHistoryLoadingBoxRef}
              flexShrink={0}
            />
            <scrollbox
              ref={props.onTranscriptScrollboxRef}
              flexGrow={1}
              stickyScroll={true}
              stickyStart="bottom"
              viewportOptions={{
                paddingRight: 0,
              }}
              verticalScrollbarOptions={{
                visible: true,
                paddingLeft: 0,
                trackOptions: {
                  backgroundColor: palette().backgroundElement,
                  foregroundColor: palette().border,
                },
              }}
            />
            <box
              ref={props.onResponsePrimaryInteractionBoxRef}
              flexShrink={0}
              flexDirection="column"
              gap={0}
              overflow="hidden"
            />
            <box
              ref={props.onResponsePrimaryFooterBoxRef}
              flexShrink={0}
              flexDirection="row"
              gap={0}
              overflow="hidden"
            />
          </box>
        )
      : (
          <box
            ref={(value) => {
              props.onResponseAuxiliaryPaneRef(paneIndex - 1, value)
            }}
            width={0}
            flexGrow={0}
            flexBasis={0}
            flexShrink={0}
            flexDirection="column"
            border={false}
            borderColor={palette().borderSubtle}
            backgroundColor={palette().backgroundElement}
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
              viewportOptions={{
                paddingRight: 0,
              }}
              verticalScrollbarOptions={{
                visible: true,
                paddingLeft: 0,
                trackOptions: {
                  backgroundColor: palette().backgroundElement,
                  foregroundColor: palette().border,
                },
              }}
            />
            <box
              ref={(value) => {
                props.onResponseAuxiliaryInteractionBoxRef(paneIndex - 1, value)
              }}
              flexShrink={0}
              flexDirection="column"
              gap={0}
              overflow="hidden"
            />
            <box
              ref={(value) => {
                props.onResponseAuxiliaryFooterBoxRef(paneIndex - 1, value)
              }}
              flexShrink={0}
              flexDirection="row"
              gap={0}
              overflow="hidden"
            />
          </box>
        )
  }

  const renderBorderRow = (
    onRowRef: (value: any) => void,
    onHorizontalRef: (segmentIndex: number, value: any) => void,
    onJunctionRef: (junctionIndex: number, value: any) => void,
  ) => (
    <box
      ref={onRowRef}
      height={0}
      minHeight={0}
      flexGrow={0}
      flexShrink={0}
      flexDirection="row"
      gap={0}
      visible={false}
    >
      <text ref={(value) => onJunctionRef(0, value)} fg={palette().borderSubtle}>{" "}</text>
      <box
        ref={(value) => onHorizontalRef(0, value)}
        height={1}
        minHeight={1}
        flexGrow={1}
        flexBasis={0}
        border={false}
        borderColor={palette().borderSubtle}
        customBorderChars={PaneGridBorderChars}
        visible={false}
      />
      <text ref={(value) => onJunctionRef(1, value)} fg={palette().borderSubtle}>{" "}</text>
      <box
        ref={(value) => onHorizontalRef(1, value)}
        height={1}
        minHeight={1}
        flexGrow={1}
        flexBasis={0}
        border={false}
        borderColor={palette().borderSubtle}
        customBorderChars={PaneGridBorderChars}
        visible={false}
      />
      <text ref={(value) => onJunctionRef(2, value)} fg={palette().borderSubtle}>{" "}</text>
    </box>
  )

  const renderVerticalSegment = (rowIndex: number, segmentIndex: number) => (
    <box
      ref={(value) => {
        props.onPaneGridVerticalSegmentRef(rowIndex, segmentIndex, value)
      }}
      width={0}
      minWidth={0}
      flexGrow={0}
      flexShrink={0}
      border={false}
      borderColor={palette().borderSubtle}
      customBorderChars={PaneGridBorderChars}
      visible={false}
    />
  )

  return (
    <box
      width={props.width}
      height={props.height}
      flexDirection="column"
      backgroundColor={palette().background}
      onMouseUp={props.onRootMouseUp}
    >
      <box
        flexGrow={1}
        backgroundColor={palette().backgroundPanel}
        onMouseUp={props.onResponseSurfaceMouseUp}
      >
        <box
          ref={props.onResponseLayoutBoxRef}
          flexGrow={1}
          flexDirection="column"
          gap={0}
        >
          <For each={props.responsePaneRows()}>
            {(rowSlots, rowIndex) => (
              <>
                {renderBorderRow(
                  (value) => {
                    props.onPaneGridBorderRowRef(rowIndex(), value)
                  },
                  (segmentIndex, value) => {
                    props.onPaneGridHorizontalSegmentRef(rowIndex(), segmentIndex, value)
                  },
                  (junctionIndex, value) => {
                    props.onPaneGridJunctionTextRef(rowIndex(), junctionIndex, value)
                  },
                )}
                <box
                  ref={(value) => {
                    props.onResponseRowBoxRef(rowIndex(), value)
                  }}
                  flexGrow={1}
                  flexDirection="row"
                  gap={0}
                >
                  {renderVerticalSegment(rowIndex(), 0)}
                  {renderPaneSlot(rowSlots[0])}
                  {renderVerticalSegment(rowIndex(), 1)}
                  {renderPaneSlot(rowSlots[1])}
                  {renderVerticalSegment(rowIndex(), 2)}
                </box>
              </>
            )}
          </For>
          {renderBorderRow(
            props.onPaneGridBottomBorderRowRef,
            props.onPaneGridBottomHorizontalSegmentRef,
            props.onPaneGridBottomJunctionTextRef,
          )}
        </box>
      </box>

      <box
        flexShrink={0}
        overflow="visible"
        border={false}
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
          backgroundColor={props.promptAreaBackground}
          flexDirection="column"
          gap={0}
          paddingTop={1}
        >
          <box overflow="hidden">
            <textarea
              ref={props.onPromptInputRef}
              placeholder={props.promptPlaceholder}
              textColor={palette().text}
              focusedTextColor={palette().text}
              minHeight={1}
              maxHeight={props.promptInputMaxHeight}
              keyBindings={props.promptKeyBindings}
              onKeyDown={props.onPromptKeyDown}
              onContentChange={props.onPromptContentChange}
              onSubmit={props.onPromptSubmit}
            />
          </box>
          <box flexDirection="row" overflow="hidden">
            <text ref={props.onPromptMetaProviderTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaProviderDividerTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaModelTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaModelDividerTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaVariantTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageDividerTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageTokensTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarOpenTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarFilledTextRef} fg={palette().primary}>{""}</text>
            <text ref={props.onPromptMetaUsageBarEmptyTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsageBarCloseTextRef} fg={palette().textMuted}>{""}</text>
            <text ref={props.onPromptMetaUsagePercentTextRef} fg={palette().textMuted}>{""}</text>
          </box>
        </box>
      </box>

      <box flexShrink={0} onMouseUp={props.onFooterMouseUp}>
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
