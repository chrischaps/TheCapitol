import { useGame } from '../contexts/GameContext'
import './PlotPanel.css'

export default function PlotPanel() {
  const { selectedPlot, claimPlot, closePlotPanel, myPlayerId, strandBalance } = useGame()

  if (!selectedPlot) {
    return null
  }

  const isOwned = !!selectedPlot.ownerId
  const isOwnedByMe = selectedPlot.ownerId === myPlayerId
  const canAfford = strandBalance >= (selectedPlot.assessedValue || 0)

  const handleClaim = () => {
    if (!isOwned) {
      claimPlot(selectedPlot.id)
    }
  }

  return (
    <div className="plot-panel">
      <div className="plot-panel-header">
        <h3>Plot Details</h3>
        <button className="close-button" onClick={closePlotPanel}>x</button>
      </div>

      <div className="plot-panel-content">
        <div className="plot-info-row">
          <span className="plot-label">Zone:</span>
          <span className="plot-value">{selectedPlot.zoneId}</span>
        </div>

        <div className="plot-info-row">
          <span className="plot-label">Size:</span>
          <span className="plot-value">{selectedPlot.sizeCategory}</span>
        </div>

        <div className="plot-info-row">
          <span className="plot-label">Type:</span>
          <span className="plot-value">{selectedPlot.plotType}</span>
        </div>

        <div className="plot-info-row">
          <span className="plot-label">Stations:</span>
          <span className="plot-value">
            {selectedPlot.stationCount} / {selectedPlot.stationCapacity}
          </span>
        </div>

        <div className="plot-info-row">
          <span className="plot-label">Owner:</span>
          <span className={`plot-value ${isOwnedByMe ? 'owner-me' : ''}`}>
            {isOwned ? (isOwnedByMe ? 'You' : selectedPlot.ownerName || 'Unknown') : 'Unclaimed'}
          </span>
        </div>

        {isOwned && (
          <div className="plot-info-row">
            <span className="plot-label">Value:</span>
            <span className="plot-value">{selectedPlot.assessedValue} Strands</span>
          </div>
        )}

        {!isOwned && (
          <div className="plot-claim-section">
            <div className="plot-deed-cost">
              Deed Cost: <strong>{selectedPlot.assessedValue || 'N/A'} Strands</strong>
            </div>
            <button
              className="claim-button"
              onClick={handleClaim}
              disabled={!canAfford}
            >
              {canAfford ? 'Claim Plot' : 'Insufficient Strands'}
            </button>
          </div>
        )}

        {isOwnedByMe && (
          <div className="plot-owner-section">
            <div className="plot-owner-info">
              You own this plot. Place stations here!
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
