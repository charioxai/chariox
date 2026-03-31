import type { AgentInstance } from './index.js';

export interface GridPosition {
  row: number;
  col: number;
  rowSpan: number;
  colSpan: number;
}

/**
 * Calculates grid layout for agents based on count.
 * Uses a dynamic 2x2 or 2x3 grid that expands as agents are added.
 * 
 * Layout progression:
 * - 1 agent: full screen (2x2)
 * - 2 agents: split vertically (1x2)
 * - 3 agents: split horizontally, leave 1 empty (2x2 with 1 slot)
 * - 4 agents: fill 2x2 grid
 * - 5 agents: add column, leave 1 empty (2x3 with 1 slot)  
 * - 6 agents: fill 2x3 grid
 */
export function calculateAgentLayout(agentCount: number): GridPosition[] {
  const positions: GridPosition[] = [];
  
  switch (agentCount) {
    case 1:
      // Full screen: spans entire 2x2 area
      positions.push({ row: 0, col: 0, rowSpan: 2, colSpan: 2 });
      break;
      
    case 2:
      // Split vertically: side by side
      positions.push({ row: 0, col: 0, rowSpan: 2, colSpan: 1 });
      positions.push({ row: 0, col: 1, rowSpan: 2, colSpan: 1 });
      break;
      
    case 3:
      // 2x2 grid with one empty slot
      positions.push({ row: 0, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 1, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 0, rowSpan: 1, colSpan: 1 });
      // (1, 1) is empty
      break;
      
    case 4:
      // Full 2x2 grid
      positions.push({ row: 0, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 1, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 1, rowSpan: 1, colSpan: 1 });
      break;
      
    case 5:
      // 2x3 grid with one empty slot
      positions.push({ row: 0, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 1, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 2, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 1, rowSpan: 1, colSpan: 1 });
      // (1, 2) is empty
      break;
      
    case 6:
      // Full 2x3 grid
      positions.push({ row: 0, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 1, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 0, col: 2, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 0, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 1, rowSpan: 1, colSpan: 1 });
      positions.push({ row: 1, col: 2, rowSpan: 1, colSpan: 1 });
      break;
      
    default:
      throw new Error(`Unsupported agent count: ${agentCount}. Max is 6.`);
  }
  
  return positions;
}

/**
 * Get the grid template dimensions for a given agent count.
 */
export function getGridDimensions(agentCount: number): { rows: number; cols: number } {
  if (agentCount <= 4) {
    return { rows: 2, cols: 2 };
  } else {
    return { rows: 2, cols: 3 };
  }
}

/**
 * Recalculate layouts for all agents when count changes.
 * Returns map of agent index to new position.
 */
export function recalculateLayouts(agentCount: number): Map<number, GridPosition> {
  const positions = calculateAgentLayout(agentCount);
  const layoutMap = new Map<number, GridPosition>();
  
  positions.forEach((pos, index) => {
    layoutMap.set(index, pos);
  });
  
  return layoutMap;
}
