# The Capitol - Development Start Script

Write-Host "Starting The Capitol Development Environment" -ForegroundColor Yellow
Write-Host ""

# Check Docker
if (!(Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Host "Error: Docker is not installed or not in PATH" -ForegroundColor Red
    exit 1
}

# Start Docker containers
Write-Host "Starting PostgreSQL and Redis..." -ForegroundColor Cyan
docker-compose up -d

# Wait for databases
Write-Host "Waiting for databases to be ready..." -ForegroundColor Cyan
Start-Sleep -Seconds 3

# Create backend .env if it doesn't exist
if (!(Test-Path "backend\.env")) {
    Write-Host "Creating backend .env from template..." -ForegroundColor Cyan
    Copy-Item "backend\.env.example" "backend\.env"
}

Write-Host ""
Write-Host "To start the backend:" -ForegroundColor Green
Write-Host "  cd backend && cargo run" -ForegroundColor White
Write-Host ""
Write-Host "To start the client (in a new terminal):" -ForegroundColor Green
Write-Host "  cd client && npm run dev" -ForegroundColor White
Write-Host ""
Write-Host "Then open http://localhost:5173 in your browser" -ForegroundColor Yellow
