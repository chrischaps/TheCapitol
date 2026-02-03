import { useState, useMemo, useCallback } from 'react'
import { useGame } from '../contexts/GameContext'
import type { DraggedItem } from '../contexts/DragDropContext'
import InventoryGrid from './InventoryGrid'
import type { SlotItem } from './InventorySlot'
import './CraftingPanel.css'

export default function CraftingPanel() {
  const {
    recipes,
    craftingInputContainer,
    craftingOutputContainer,
    craftingState,
    isCrafting,
    startCrafting,
    cancelCrafting,
    moveItem,
  } = useGame()

  const [selectedRecipeId, setSelectedRecipeId] = useState<string | null>(null)

  const selectedRecipe = useMemo(() => {
    if (!selectedRecipeId) return null
    return recipes.find(r => r.id === selectedRecipeId) || null
  }, [recipes, selectedRecipeId])

  // Convert container slots to component format
  const inputSlots = useMemo(() => {
    const slots = new Map<number, SlotItem>()
    if (craftingInputContainer) {
      for (const [idx, item] of craftingInputContainer.slots) {
        slots.set(idx, {
          itemId: item.itemId,
          itemType: item.itemType,
          itemName: item.itemName,
          quality: item.quality,
          qualityGrade: item.qualityGrade,
          quantity: item.quantity,
          slotIndex: item.slotIndex,
        })
      }
    }
    return slots
  }, [craftingInputContainer])

  const outputSlots = useMemo(() => {
    const slots = new Map<number, SlotItem>()
    if (craftingOutputContainer) {
      for (const [idx, item] of craftingOutputContainer.slots) {
        slots.set(idx, {
          itemId: item.itemId,
          itemType: item.itemType,
          itemName: item.itemName,
          quality: item.quality,
          qualityGrade: item.qualityGrade,
          quantity: item.quantity,
          slotIndex: item.slotIndex,
        })
      }
    }
    return slots
  }, [craftingOutputContainer])

  // Detect recipe from input slot contents (finds best match)
  const detectedRecipe = useMemo(() => {
    if (inputSlots.size === 0) return null

    // Gather items in input slots by type
    const inputCounts = new Map<string, number>()
    for (const item of inputSlots.values()) {
      const current = inputCounts.get(item.itemType) || 0
      inputCounts.set(item.itemType, current + item.quantity)
    }

    // Find all matching recipes and score them
    type ScoredRecipe = { recipe: typeof recipes[0]; score: number; exactMatch: boolean }
    const matches: ScoredRecipe[] = []

    for (const recipe of recipes) {
      let canCraft = true
      let totalRequired = 0
      let isExact = true

      for (const input of recipe.inputs) {
        const have = inputCounts.get(input.item_type) || 0
        if (have < input.quantity) {
          canCraft = false
          break
        }
        totalRequired += input.quantity
        if (have !== input.quantity) {
          isExact = false
        }
      }

      if (canCraft) {
        matches.push({ recipe, score: totalRequired, exactMatch: isExact })
      }
    }

    if (matches.length === 0) return null

    // Prefer exact matches, then highest material consumption
    matches.sort((a, b) => {
      if (a.exactMatch && !b.exactMatch) return -1
      if (!a.exactMatch && b.exactMatch) return 1
      return b.score - a.score // Higher score (more materials) wins
    })

    return matches[0].recipe
  }, [inputSlots, recipes])

  // Active recipe: manual selection takes priority over auto-detection
  const activeRecipe = selectedRecipe || detectedRecipe

  // Calculate expected output quality
  const expectedQuality = useMemo(() => {
    if (!activeRecipe || inputSlots.size === 0) return null

    let totalQuality = 0
    let totalCount = 0
    for (const item of inputSlots.values()) {
      totalQuality += item.quality * item.quantity
      totalCount += item.quantity
    }
    return totalCount > 0 ? Math.round(totalQuality / totalCount) : 0
  }, [activeRecipe, inputSlots])

  // Calculate ingredient status for the active recipe
  const ingredientStatus = useMemo(() => {
    if (!activeRecipe) return null

    // Count items in input slots by type
    const inputCounts = new Map<string, number>()
    for (const item of inputSlots.values()) {
      const current = inputCounts.get(item.itemType) || 0
      inputCounts.set(item.itemType, current + item.quantity)
    }

    return activeRecipe.inputs.map(input => ({
      itemType: input.item_type,
      itemName: input.item_name,
      required: input.quantity,
      have: inputCounts.get(input.item_type) || 0,
    }))
  }, [detectedRecipe, selectedRecipe, inputSlots])

  // Get items from input slots for crafting
  const getInputItemIds = (): string[] => {
    const ids: string[] = []
    for (const item of inputSlots.values()) {
      // Add item id repeated by quantity (for the backend)
      for (let i = 0; i < item.quantity; i++) {
        ids.push(item.itemId)
      }
    }
    return ids
  }

  // Check if we can craft the active recipe
  const canCraft = useMemo(() => {
    if (!activeRecipe || !ingredientStatus) return false
    return ingredientStatus.every(ing => ing.have >= ing.required)
  }, [activeRecipe, ingredientStatus])

  const handleCraft = () => {
    if (!activeRecipe || !canCraft) return
    const inputItemIds = getInputItemIds()
    startCrafting(activeRecipe.id, inputItemIds)
  }

  const handleItemDrop = useCallback((targetContainer: string, targetSlot: number, item: DraggedItem) => {
    if (item.sourceContainer === targetContainer && item.sourceSlot === targetSlot) {
      return
    }
    moveItem(item.itemId, targetContainer, targetSlot)
  }, [moveItem])

  const craftingProgress = craftingState
    ? Math.round((craftingState.progress / craftingState.duration) * 100)
    : 0

  const qualityGrade = (quality: number): string => {
    if (quality >= 90) return 'A'
    if (quality >= 75) return 'B'
    if (quality >= 55) return 'C'
    if (quality >= 35) return 'D'
    return 'F'
  }

  return (
    <div className="crafting-panel-grid">
      <div className="crafting-header">
        <h3>Crafting</h3>
        {isCrafting && <span className="crafting-active">In Progress</span>}
      </div>

      {isCrafting && craftingState ? (
        <div className="crafting-in-progress">
          <p className="crafting-status">Crafting {craftingState.recipeName}...</p>
          <div className="crafting-progress-bar">
            <div
              className="crafting-progress-fill"
              style={{ width: `${craftingProgress}%` }}
            />
          </div>
          <p className="crafting-progress-text">{craftingProgress}%</p>
          <button className="crafting-cancel-btn" onClick={cancelCrafting}>
            Cancel
          </button>
        </div>
      ) : (
        <>
          {/* Recipe selector */}
          <div className="recipe-selector">
            <label>Recipe:</label>
            <select
              value={selectedRecipeId || ''}
              onChange={(e) => setSelectedRecipeId(e.target.value || null)}
            >
              <option value="">
                {detectedRecipe ? `Auto: ${detectedRecipe.name}` : 'Select or auto-detect'}
              </option>
              {recipes.map((recipe) => (
                <option key={recipe.id} value={recipe.id}>
                  {recipe.name}
                </option>
              ))}
            </select>
          </div>

          {/* Ingredient requirements */}
          {ingredientStatus && ingredientStatus.length > 0 && (
            <div className="ingredient-requirements">
              <span className="requirements-label">Required:</span>
              <ul className="ingredient-list">
                {ingredientStatus.map((ing) => {
                  const isSatisfied = ing.have >= ing.required
                  return (
                    <li key={ing.itemType} className={`ingredient-item ${isSatisfied ? 'satisfied' : 'missing'}`}>
                      <span className="ingredient-name">{ing.itemName}</span>
                      <span className="ingredient-count">
                        <span className={ing.have >= ing.required ? 'count-ok' : 'count-low'}>{ing.have}</span>
                        <span className="count-separator">/</span>
                        <span className="count-required">{ing.required}</span>
                      </span>
                      <span className="ingredient-status">{isSatisfied ? '✓' : '✗'}</span>
                    </li>
                  )
                })}
              </ul>
            </div>
          )}

          {/* Input/Output grid layout */}
          <div className="crafting-slots-layout">
            <div className="crafting-inputs-section">
              <span className="section-label">Inputs</span>
              {craftingInputContainer ? (
                <InventoryGrid
                  containerId={craftingInputContainer.id}
                  slotCount={craftingInputContainer.slotCount}
                  columns={craftingInputContainer.layoutColumns}
                  slots={inputSlots}
                  onItemDrop={handleItemDrop}
                />
              ) : (
                <div className="loading-slots">Loading...</div>
              )}
            </div>

            <div className="crafting-arrow">→</div>

            <div className="crafting-output-section">
              <span className="section-label">Output</span>
              {craftingOutputContainer ? (
                <InventoryGrid
                  containerId={craftingOutputContainer.id}
                  slotCount={craftingOutputContainer.slotCount}
                  columns={craftingOutputContainer.layoutColumns}
                  slots={outputSlots}
                  onItemDrop={handleItemDrop}
                />
              ) : (
                <div className="loading-slots">Loading...</div>
              )}
            </div>
          </div>

          {/* Quality preview */}
          {canCraft && expectedQuality !== null && activeRecipe && (
            <div className="quality-preview">
              <span className="preview-label">Preview:</span>
              <span className="preview-output">
                {activeRecipe.outputs[0]?.item_name || 'Output'}
              </span>
              <span className={`preview-quality grade-${qualityGrade(expectedQuality).toLowerCase()}`}>
                ~{expectedQuality} {qualityGrade(expectedQuality)}
              </span>
            </div>
          )}

          {/* Craft button */}
          <button
            className="craft-button"
            onClick={handleCraft}
            disabled={!canCraft}
          >
            {activeRecipe
              ? (canCraft ? `Craft ${activeRecipe.name}` : `Need materials for ${activeRecipe.name}`)
              : 'Select a recipe'}
          </button>

          {/* Recipe info */}
          {activeRecipe && (
            <div className="recipe-info">
              <h4>{activeRecipe.name}</h4>
              <p className="recipe-time">
                Time: {(activeRecipe.duration_ticks || 0) / 10}s
              </p>
            </div>
          )}
        </>
      )}
    </div>
  )
}
