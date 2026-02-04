import { useState, useMemo } from 'react'
import { useGame, type SlotItem } from '../contexts/GameContext'
import './TradeWindow.css'

interface OfferItem {
  itemId: string
  quantity: number
}

export default function TradeWindow() {
  const {
    activeTrade,
    inventoryContainer,
    strandBalance,
    updateTradeOffer,
    acceptTrade,
    cancelTrade,
  } = useGame()

  const [myOfferItems, setMyOfferItems] = useState<OfferItem[]>([])
  const [myOfferStrands, setMyOfferStrands] = useState('')
  const [selectedInventoryItem, setSelectedInventoryItem] = useState<string | null>(null)
  const [addQuantity, setAddQuantity] = useState('1')

  if (!activeTrade) return null

  const inventoryItems = useMemo(() => {
    if (!inventoryContainer) return []
    return Array.from(inventoryContainer.slots.values())
  }, [inventoryContainer])

  const myStrandsNum = parseInt(myOfferStrands) || 0

  const addItemToOffer = () => {
    if (!selectedInventoryItem) return
    const item = inventoryItems.find((i) => i.itemId === selectedInventoryItem)
    if (!item) return

    const qty = Math.min(parseInt(addQuantity) || 1, item.quantity)
    if (qty <= 0) return

    setMyOfferItems((prev) => {
      // Check if item already in offer
      const existing = prev.find((o) => o.itemId === selectedInventoryItem)
      if (existing) {
        // Increase quantity up to max
        const newQty = Math.min(existing.quantity + qty, item.quantity)
        return prev.map((o) =>
          o.itemId === selectedInventoryItem ? { ...o, quantity: newQty } : o
        )
      }
      return [...prev, { itemId: selectedInventoryItem, quantity: qty }]
    })
    setSelectedInventoryItem(null)
    setAddQuantity('1')
  }

  const removeItemFromOffer = (itemId: string) => {
    setMyOfferItems((prev) => prev.filter((o) => o.itemId !== itemId))
  }

  const handleUpdateOffer = () => {
    updateTradeOffer(myOfferItems, myStrandsNum)
  }

  const getItemInfo = (itemId: string): SlotItem | undefined => {
    return inventoryItems.find((i) => i.itemId === itemId)
  }

  return (
    <div className="trade-window-overlay">
      <div className="trade-window">
        <div className="trade-header">
          <h3>Trading with {activeTrade.partnerName}</h3>
          <button className="cancel-button" onClick={cancelTrade}>
            Cancel
          </button>
        </div>

        <div className="trade-content">
          {/* My Offer */}
          <div className="trade-side my-offer">
            <h4>Your Offer</h4>

            <div className="offer-items">
              {myOfferItems.length === 0 ? (
                <p className="no-items">No items in offer</p>
              ) : (
                myOfferItems.map((offer) => {
                  const item = getItemInfo(offer.itemId)
                  return (
                    <div key={offer.itemId} className="offer-item">
                      <span className="item-name">{item?.itemName || 'Unknown'}</span>
                      <span className="item-qty">x{offer.quantity}</span>
                      <button
                        className="remove-item"
                        onClick={() => removeItemFromOffer(offer.itemId)}
                      >
                        -
                      </button>
                    </div>
                  )
                })
              )}
            </div>

            <div className="add-item-section">
              <select
                value={selectedInventoryItem || ''}
                onChange={(e) => setSelectedInventoryItem(e.target.value || null)}
              >
                <option value="">Select item...</option>
                {inventoryItems.map((item) => (
                  <option key={item.itemId} value={item.itemId}>
                    {item.itemName} x{item.quantity} [{item.qualityGrade}]
                  </option>
                ))}
              </select>
              <input
                type="number"
                min="1"
                value={addQuantity}
                onChange={(e) => setAddQuantity(e.target.value)}
                placeholder="Qty"
                className="qty-input"
              />
              <button onClick={addItemToOffer} disabled={!selectedInventoryItem}>
                Add
              </button>
            </div>

            <div className="strands-section">
              <label>Strands to offer:</label>
              <input
                type="number"
                min="0"
                max={strandBalance}
                value={myOfferStrands}
                onChange={(e) => setMyOfferStrands(e.target.value)}
                placeholder="0"
              />
              <span className="balance-hint">
                (Balance: {strandBalance.toLocaleString()})
              </span>
            </div>

            <button className="update-offer-button" onClick={handleUpdateOffer}>
              Update Offer
            </button>

            <div className={`accepted-status ${activeTrade.iAccepted ? 'accepted' : ''}`}>
              {activeTrade.iAccepted ? 'You accepted' : 'You have not accepted'}
            </div>
          </div>

          {/* Their Offer */}
          <div className="trade-side their-offer">
            <h4>Their Offer</h4>

            <div className="offer-items">
              {activeTrade.theirOffer.items.length === 0 ? (
                <p className="no-items">No items in offer</p>
              ) : (
                activeTrade.theirOffer.items.map((item) => (
                  <div key={item.itemId} className="offer-item">
                    <span className="item-name">{item.itemName}</span>
                    <span className="item-qty">x{item.quantity}</span>
                    <span className="item-quality">[Q{item.quality}]</span>
                  </div>
                ))
              )}
            </div>

            {activeTrade.theirOffer.strands > 0 && (
              <div className="their-strands">
                <span className="strands-label">Strands:</span>
                <span className="strands-amount">
                  {activeTrade.theirOffer.strands.toLocaleString()}
                </span>
              </div>
            )}

            <div className={`accepted-status ${activeTrade.theyAccepted ? 'accepted' : ''}`}>
              {activeTrade.theyAccepted ? 'Partner accepted' : 'Partner has not accepted'}
            </div>
          </div>
        </div>

        <div className="trade-footer">
          <button
            className="accept-trade-button"
            onClick={acceptTrade}
            disabled={activeTrade.iAccepted}
          >
            {activeTrade.iAccepted ? 'Waiting for partner...' : 'Accept Trade'}
          </button>
          {activeTrade.iAccepted && activeTrade.theyAccepted && (
            <p className="executing">Executing trade...</p>
          )}
        </div>
      </div>
    </div>
  )
}
