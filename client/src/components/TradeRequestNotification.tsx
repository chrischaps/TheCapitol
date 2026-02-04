import { useGame } from '../contexts/GameContext'
import './TradeRequestNotification.css'

export default function TradeRequestNotification() {
  const { tradeRequest, acceptTradeRequest, declineTradeRequest } = useGame()

  if (!tradeRequest) return null

  return (
    <div className="trade-request-notification">
      <div className="trade-request-content">
        <p className="trade-request-message">
          <span className="player-name">{tradeRequest.fromPlayerName}</span> wants to trade with you
        </p>
        <div className="trade-request-actions">
          <button className="accept-button" onClick={acceptTradeRequest}>
            Accept
          </button>
          <button className="decline-button" onClick={declineTradeRequest}>
            Decline
          </button>
        </div>
      </div>
    </div>
  )
}
