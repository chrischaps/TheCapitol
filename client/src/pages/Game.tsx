import { useAuth } from '../contexts/AuthContext'
import { GameProvider, useGame } from '../contexts/GameContext'
import { DragDropProvider } from '../contexts/DragDropContext'
import GameCanvas from '../components/GameCanvas'
import InventoryPanel from '../components/InventoryPanel'
import CraftingPanel from '../components/CraftingPanel'
import StationPanel from '../components/StationPanel'
import ExchangePanel from '../components/ExchangePanel'
import TradeRequestNotification from '../components/TradeRequestNotification'
import TradeWindow from '../components/TradeWindow'
import ZoneIndicator from '../components/ZoneIndicator'
import PlotPanel from '../components/PlotPanel'
import './Game.css'

function CurrencyDisplay() {
  const { strandBalance } = useGame()
  return (
    <span className="currency-display">
      <span className="currency-icon">S</span>
      <span className="currency-amount">{strandBalance.toLocaleString()}</span>
    </span>
  )
}

function GameContent() {
  const { logout, email } = useAuth()
  const { tradeRequest, activeTrade, selectedPlot } = useGame()

  return (
    <div className="game-container">
      <header className="game-header">
        <h1 className="game-title">The Capitol</h1>
        <div className="header-right">
          <ZoneIndicator />
          <CurrencyDisplay />
          <span className="user-email">{email}</span>
          <button onClick={logout} className="logout-button">
            Logout
          </button>
        </div>
      </header>
      <main className="game-main">
        <GameCanvas />
        <InventoryPanel />
        <CraftingPanel />
        <StationPanel />
        <ExchangePanel />
        {tradeRequest && <TradeRequestNotification />}
        {activeTrade && <TradeWindow />}
        {selectedPlot && <PlotPanel />}
      </main>
    </div>
  )
}

export default function Game() {
  return (
    <GameProvider>
      <DragDropProvider>
        <GameContent />
      </DragDropProvider>
    </GameProvider>
  )
}
