import { useAuth } from '../contexts/AuthContext'
import { GameProvider } from '../contexts/GameContext'
import { DragDropProvider } from '../contexts/DragDropContext'
import GameCanvas from '../components/GameCanvas'
import InventoryPanel from '../components/InventoryPanel'
import CraftingPanel from '../components/CraftingPanel'
import StationPanel from '../components/StationPanel'
import './Game.css'

export default function Game() {
  const { logout, email } = useAuth()

  return (
    <GameProvider>
      <DragDropProvider>
        <div className="game-container">
          <header className="game-header">
            <h1 className="game-title">The Capitol</h1>
            <div className="header-right">
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
          </main>
        </div>
      </DragDropProvider>
    </GameProvider>
  )
}
