import { useState, useMemo, useCallback } from 'react'
import { useGame } from '../contexts/GameContext'
import type { DraggedItem } from '../contexts/DragDropContext'
import InventoryGrid from './InventoryGrid'
import type { SlotItem } from './InventorySlot'
import './StationPanel.css'

export default function StationPanel() {
  const {
    openStation,
    openStationContainer,
    recipes,
    craftingState,
    isCrafting,
    closeStation,
    craftAtStation,
    cancelCrafting,
    moveItem,
  } = useGame()

  const [selectedRecipeId, setSelectedRecipeId] = useState<string | null>(null)

  // Filter recipes that can be crafted at this station
  const stationRecipes = useMemo(() => {
    if (!openStation) return []
    return recipes.filter(r => r.required_station_type === openStation.stationType)
  }, [recipes, openStation])

  const selectedRecipe = useMemo(() => {
    if (!selectedRecipeId) return null
    return stationRecipes.find(r => r.id === selectedRecipeId) || null
  }, [stationRecipes, selectedRecipeId])

  // Convert container slots to component format
  const stationSlots = useMemo(() => {
    const slots = new Map<number, SlotItem>()
    if (openStationContainer) {
      for (const [idx, item] of openStationContainer.slots) {
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
  }, [openStationContainer])

  // Detect recipe from station contents
  const detectedRecipe = useMemo(() => {
    if (stationSlots.size === 0 || !openStation) return null

    // Gather items in station
    const stationItems: { type: string; quantity: number }[] = []
    for (const item of stationSlots.values()) {
      stationItems.push({ type: item.itemType, quantity: item.quantity })
    }

    // Check each station recipe
    for (const recipe of stationRecipes) {
      let matches = true
      for (const input of recipe.inputs) {
        const matching = stationItems.filter(i => i.type === input.item_type)
        const totalQty = matching.reduce((sum, i) => sum + i.quantity, 0)
        if (totalQty < input.quantity) {
          matches = false
          break
        }
      }
      if (matches) {
        return recipe
      }
    }
    return null
  }, [stationSlots, stationRecipes, openStation])

  // Calculate expected output quality
  const expectedQuality = useMemo(() => {
    if (!detectedRecipe || stationSlots.size === 0) return null

    let totalQuality = 0
    let totalCount = 0
    for (const item of stationSlots.values()) {
      totalQuality += item.quality * item.quantity
      totalCount += item.quantity
    }
    return totalCount > 0 ? Math.round(totalQuality / totalCount) : 0
  }, [detectedRecipe, stationSlots])

  // Get items from station for crafting
  const getInputItemIds = (): string[] => {
    const ids: string[] = []
    for (const item of stationSlots.values()) {
      for (let i = 0; i < item.quantity; i++) {
        ids.push(item.itemId)
      }
    }
    return ids
  }

  const handleCraft = () => {
    if (!detectedRecipe || !openStation) return
    const inputItemIds = getInputItemIds()
    craftAtStation(openStation.id, detectedRecipe.id, inputItemIds)
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

  const getStationIcon = (stationType: string): string => {
    switch (stationType) {
      case 'workbench': return '\u{1F6E0}'  // hammer and wrench
      case 'forge': return '\u{1F525}'      // fire
      case 'storage_chest': return '\u{1F4E6}' // package
      default: return '\u{1F3ED}'           // factory
    }
  }

  if (!openStation) return null

  const isStorage = openStation.stationType === 'storage_chest'

  return (
    <div className="station-panel-overlay">
      <div className="station-panel">
        <div className="station-header">
          <span className="station-icon">{getStationIcon(openStation.stationType)}</span>
          <h3>{openStation.name}</h3>
          <button className="station-close-btn" onClick={closeStation}>X</button>
        </div>

        <div className="station-content">
          {/* Station inventory */}
          <div className="station-inventory-section">
            <span className="section-label">
              {isStorage ? 'Storage' : 'Station Inventory'}
            </span>
            {openStationContainer ? (
              <InventoryGrid
                containerId={openStationContainer.id}
                slotCount={openStationContainer.slotCount}
                columns={openStationContainer.layoutColumns}
                slots={stationSlots}
                onItemDrop={handleItemDrop}
              />
            ) : (
              <div className="loading-slots">Loading...</div>
            )}
          </div>

          {/* Crafting section (only for crafting stations) */}
          {!isStorage && (
            <div className="station-crafting-section">
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
                  <div className="station-recipes">
                    <span className="section-label">Recipes</span>
                    {stationRecipes.length === 0 ? (
                      <p className="no-recipes">No recipes available at this station</p>
                    ) : (
                      <div className="recipe-list">
                        {stationRecipes.map((recipe) => (
                          <button
                            key={recipe.id}
                            className={`recipe-btn ${detectedRecipe?.id === recipe.id ? 'detected' : ''} ${selectedRecipeId === recipe.id ? 'selected' : ''}`}
                            onClick={() => setSelectedRecipeId(recipe.id)}
                          >
                            {recipe.name}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Quality preview */}
                  {detectedRecipe && expectedQuality !== null && (
                    <div className="quality-preview">
                      <span className="preview-label">Preview:</span>
                      <span className="preview-output">
                        {detectedRecipe.outputs[0]?.item_name || 'Output'}
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
                    disabled={!detectedRecipe}
                  >
                    {detectedRecipe ? `Craft ${detectedRecipe.name}` : 'Add materials to craft'}
                  </button>

                  {/* Recipe info */}
                  {(detectedRecipe || selectedRecipe) && (
                    <div className="recipe-info">
                      <h4>{(detectedRecipe || selectedRecipe)?.name}</h4>
                      <p className="recipe-time">
                        Time: {((detectedRecipe || selectedRecipe)?.duration_ticks || 0) / 10}s
                      </p>
                      <div className="recipe-requirements">
                        <span>Requires:</span>
                        <ul>
                          {(detectedRecipe || selectedRecipe)?.inputs.map((input, i) => (
                            <li key={i}>{input.item_name} x{input.quantity}</li>
                          ))}
                        </ul>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </div>

        <div className="station-footer">
          <p className="station-hint">
            {isStorage
              ? 'Drag items to store or retrieve'
              : 'Drag items into station, select recipe, click Craft'}
          </p>
        </div>
      </div>
    </div>
  )
}
