# start-agent.ps1 - lance l'agent Remote sur CE PC (celui qui sera controle), sans rien retaper.
#
# Ce que le script fait a ta place :
#   - se place dans le dossier remote-agent (cargo run exige d'etre dans le dossier du Cargo.toml) ;
#   - lit la cle d'API (API_AUTH_TOKEN) dans backend\.env, sauf si tu la donnes avec -ApiKey ;
#   - verifie que Rust (cargo) est installe et explique quoi faire sinon ;
#   - lance l'agent avec la souris et le clavier autorises.
#
# Pas d'emoji ni d'accent dans ce fichier volontairement : Windows PowerShell 5.1 lit parfois un .ps1 avec l'encodage de la console
# plutot que l'UTF-8 reel du fichier (meme precaution que start-dev.ps1).
#
# Usage (depuis n'importe quel dossier) :
#   .\remote-agent\start-agent.ps1
#   .\remote-agent\start-agent.ps1 -Server http://192.168.1.20:8000 -Name "PC du bureau"
#   .\remote-agent\start-agent.ps1 -Allow ""          # regarder seulement, sans souris ni clavier
#   .\remote-agent\start-agent.ps1 -Dev               # compilation plus rapide (version non optimisee)

param(
    [string]$Server = "http://localhost:8000",
    [string]$ApiKey = "",
    [string]$Allow = "mouse,keyboard",
    [string]$Name = "",
    [switch]$Dev,
    [switch]$AutoAccept
)

$ErrorActionPreference = "Stop"
$agentDir = $PSScriptRoot
$root = Split-Path -Parent $agentDir

# --- Cle d'API : parametre, sinon backend\.env ---
if (-not $ApiKey) {
    $envFile = Join-Path $root "backend\.env"
    if (Test-Path $envFile) {
        foreach ($line in Get-Content $envFile) {
            if ($line -match '^\s*API_AUTH_TOKEN\s*=\s*(.*?)\s*$') {
                $ApiKey = $Matches[1].Trim('"').Trim("'")
                break
            }
        }
    }
}
if ($ApiKey) { Write-Host "[agent] cle d'API : lue ($($ApiKey.Length) caracteres)" }
else { Write-Host "[agent] pas de cle d'API (backend sans API_AUTH_TOKEN) : ok si le backend n'en exige pas" }

# --- Rust ---
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) { $env:PATH = "$cargoBin;$env:PATH" }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "Rust (cargo) n'est pas installe. Installe-le une fois :"
    Write-Host "  1. https://rustup.rs  (rustup-init.exe, options par defaut)"
    Write-Host "  2. si on te le demande : 'Visual Studio Build Tools' avec 'Desktop development with C++'"
    Write-Host "  3. ferme et rouvre le terminal, puis relance ce script."
    exit 1
}

# --- Lancement ---
$cargoArgs = @("run")
if (-not $Dev) { $cargoArgs += "--release" }
$cargoArgs += @("--", "--server", $Server, "--allow", $Allow)
if ($ApiKey) { $cargoArgs += @("--api-key", $ApiKey) }
if ($Name) { $cargoArgs += @("--name", $Name) }
if ($AutoAccept) { $cargoArgs += "--auto-accept" }

Write-Host "[agent] serveur : $Server | permissions en plus de la vue : '$Allow'"
Write-Host "[agent] la premiere compilation prend quelques minutes ; ensuite c'est instantane."
Push-Location $agentDir
try { & cargo @cargoArgs } finally { Pop-Location }
