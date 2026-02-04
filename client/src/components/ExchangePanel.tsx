import { useState, useMemo } from 'react'
import { useGame } from '../contexts/GameContext'
import { useAuth } from '../contexts/AuthContext'
import { depositFiber, withdrawFiber } from '../api/exchange'
import './ExchangePanel.css'

const EXCHANGE_FEE_RATE = 0.005

export default function ExchangePanel() {
  const { token } = useAuth()
  const { inventoryContainer, strandBalance, refreshCurrency, refreshContainers, refreshInventory } = useGame()
  const [isOpen, setIsOpen] = useState(false)
  const [activeTab, setActiveTab] = useState<'deposit' | 'withdraw'>('deposit')
  const [selectedFiberItems, setSelectedFiberItems] = useState<Set<string>>(new Set())
  const [withdrawAmount, setWithdrawAmount] = useState('')
  const [isProcessing, setIsProcessing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Find fiber items in inventory
  const fiberItems = useMemo(() => {
    if (!inventoryContainer) return []
    return Array.from(inventoryContainer.slots.values()).filter(
      (slot) => slot.itemType === 'fiber'
    )
  }, [inventoryContainer])

  const totalSelectedFiber = useMemo(() => {
    let total = 0
    for (const itemId of selectedFiberItems) {
      const item = fiberItems.find((f) => f.itemId === itemId)
      if (item) total += item.quantity
    }
    return total
  }, [selectedFiberItems, fiberItems])

  const depositFee = Math.floor(totalSelectedFiber * EXCHANGE_FEE_RATE)
  const strandsToReceive = totalSelectedFiber - depositFee

  const withdrawNum = parseInt(withdrawAmount) || 0
  const strandsNeeded = Math.ceil(withdrawNum / (1 - EXCHANGE_FEE_RATE))
  const withdrawFee = strandsNeeded - withdrawNum

  const toggleFiberItem = (itemId: string) => {
    setSelectedFiberItems((prev) => {
      const next = new Set(prev)
      if (next.has(itemId)) {
        next.delete(itemId)
      } else {
        next.add(itemId)
      }
      return next
    })
  }

  const selectAllFiber = () => {
    setSelectedFiberItems(new Set(fiberItems.map((f) => f.itemId)))
  }

  const handleDeposit = async () => {
    if (!token || selectedFiberItems.size === 0) return
    setIsProcessing(true)
    setError(null)
    try {
      await depositFiber(token, Array.from(selectedFiberItems))
      setSelectedFiberItems(new Set())
      refreshCurrency()
      refreshContainers()
      refreshInventory()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Deposit failed')
    } finally {
      setIsProcessing(false)
    }
  }

  const handleWithdraw = async () => {
    if (!token || withdrawNum <= 0) return
    if (strandsNeeded > strandBalance) {
      setError('Insufficient strands')
      return
    }
    setIsProcessing(true)
    setError(null)
    try {
      await withdrawFiber(token, withdrawNum)
      setWithdrawAmount('')
      refreshCurrency()
      refreshContainers()
      refreshInventory()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Withdraw failed')
    } finally {
      setIsProcessing(false)
    }
  }

  if (!isOpen) {
    return (
      <button className="exchange-toggle-button" onClick={() => setIsOpen(true)}>
        Exchange
      </button>
    )
  }

  return (
    <div className="exchange-panel">
      <div className="exchange-header">
        <h3>Fiber Exchange</h3>
        <button className="close-button" onClick={() => setIsOpen(false)}>
          X
        </button>
      </div>

      <div className="exchange-tabs">
        <button
          className={`tab ${activeTab === 'deposit' ? 'active' : ''}`}
          onClick={() => setActiveTab('deposit')}
        >
          Deposit
        </button>
        <button
          className={`tab ${activeTab === 'withdraw' ? 'active' : ''}`}
          onClick={() => setActiveTab('withdraw')}
        >
          Withdraw
        </button>
      </div>

      <div className="exchange-balance">
        Your Balance: <span className="balance-amount">{strandBalance.toLocaleString()}</span> Strands
      </div>

      {error && <div className="exchange-error">{error}</div>}

      {activeTab === 'deposit' ? (
        <div className="exchange-content">
          <p className="exchange-info">
            Convert fiber to Strands at 1:1 ratio (0.5% fee)
          </p>

          {fiberItems.length === 0 ? (
            <p className="no-items">No fiber in inventory</p>
          ) : (
            <>
              <div className="fiber-list">
                {fiberItems.map((item) => (
                  <label key={item.itemId} className="fiber-item">
                    <input
                      type="checkbox"
                      checked={selectedFiberItems.has(item.itemId)}
                      onChange={() => toggleFiberItem(item.itemId)}
                    />
                    <span className="fiber-name">Fiber</span>
                    <span className="fiber-qty">x{item.quantity}</span>
                    <span className="fiber-quality">[{item.qualityGrade}]</span>
                  </label>
                ))}
              </div>
              <button className="select-all" onClick={selectAllFiber}>
                Select All
              </button>
            </>
          )}

          {totalSelectedFiber > 0 && (
            <div className="exchange-summary">
              <div className="summary-row">
                <span>Fiber to deposit:</span>
                <span>{totalSelectedFiber}</span>
              </div>
              <div className="summary-row fee">
                <span>Fee (0.5%):</span>
                <span>-{depositFee}</span>
              </div>
              <div className="summary-row total">
                <span>You receive:</span>
                <span>{strandsToReceive} Strands</span>
              </div>
            </div>
          )}

          <button
            className="exchange-action"
            disabled={totalSelectedFiber <= 0 || isProcessing}
            onClick={handleDeposit}
          >
            {isProcessing ? 'Processing...' : 'Deposit Fiber'}
          </button>
        </div>
      ) : (
        <div className="exchange-content">
          <p className="exchange-info">
            Convert Strands back to fiber (0.5% fee)
          </p>

          <div className="withdraw-input-group">
            <label>Amount of fiber to receive:</label>
            <input
              type="number"
              min="1"
              value={withdrawAmount}
              onChange={(e) => setWithdrawAmount(e.target.value)}
              placeholder="Enter amount"
            />
          </div>

          {withdrawNum > 0 && (
            <div className="exchange-summary">
              <div className="summary-row">
                <span>Fiber to receive:</span>
                <span>{withdrawNum}</span>
              </div>
              <div className="summary-row fee">
                <span>Fee (0.5%):</span>
                <span>+{withdrawFee}</span>
              </div>
              <div className="summary-row total">
                <span>Total cost:</span>
                <span>{strandsNeeded} Strands</span>
              </div>
              {strandsNeeded > strandBalance && (
                <div className="insufficient">Insufficient strands!</div>
              )}
            </div>
          )}

          <button
            className="exchange-action"
            disabled={withdrawNum <= 0 || strandsNeeded > strandBalance || isProcessing}
            onClick={handleWithdraw}
          >
            {isProcessing ? 'Processing...' : 'Withdraw Fiber'}
          </button>
        </div>
      )}
    </div>
  )
}
