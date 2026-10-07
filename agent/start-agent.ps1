# start-agent.ps1 - lance l'agent Remote sur le PC A CONTROLER (le PC "asservi"), sans rien retaper.
#
# Deux situations :
#   1. Ce PC n'a QUE l'agent (cas normal) : au premier lancement la fenetre de l'agent demande l'adresse du serveur (serveur de rendez-vous
#      pour Internet, ou PC Assistant sur le meme reseau, ex. 192.168.1.20:8000) et sa cle, puis les retient.
#   2. Ce PC est aussi celui de PC Assistant (test) : la cle d'API est lue dans backend\.env si ce fichier existe.
#
# Si un fichier remote-agent.exe est a cote de ce script (ou dans target\release), il est lance directement : Rust n'est alors pas necessaire
# sur ce PC. Sinon le script compile avec cargo (Rust requis).
#
# Pas d'emoji ni d'accent dans ce fichier volontairement : Windows PowerShell 5.1 lit parfois un .ps1 avec l'encodage de la console
# plutot que l'UTF-8 reel du fichier (meme precaution que start-dev.ps1).
#
# Usage :
#   .\start-agent.ps1                                    # apres le premier lancement
#   .\start-agent.ps1 -Server 192.168.1.20:8000 -ApiKey <cle>   # pour (re)definir l'adresse et la cle
#   .\start-agent.ps1 -Allow "mouse,keyboard"           # autoriser souris et clavier (retenu ; modifiable dans la fenetre)
#   .\start-agent.ps1 -Name "PC du salon"                # nom affiche dans PC Assistant
#   .\start-agent.ps1 -Dev                               # compilation plus rapide (sans exe)

param(
    [string]$Server = "",
    [string]$ApiKey = "",
    [string]$Allow = "",
    [string]$Name = "",
    [switch]$Dev,
    [switch]$AutoAccept
)

$ErrorActionPreference = "Stop"
$agentDir = $PSScriptRoot
$root = Split-Path -Parent $agentDir

# --- Cle d'API : parametre, sinon backend\.env s'il existe (PC de PC Assistant) ; sinon l'agent la demande et la retient ---
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

# --- Arguments : seulement ce qui est explicite, pour ne jamais ecraser l'adresse deja retenue ---
$agentArgs = @()
if ($PSBoundParameters.ContainsKey("Allow")) { $agentArgs += @("--allow", $Allow) }   # sinon : reglage retenu, modifiable dans la fenetre
if ($Server) { $agentArgs += @("--server", $Server) }
if ($ApiKey) { $agentArgs += @("--api-key", $ApiKey) }
if ($Name) { $agentArgs += @("--name", $Name) }
if ($AutoAccept) { $agentArgs += "--auto-accept" }

# --- Exe deja compile : pas besoin de Rust ---
$exe = $null
foreach ($candidate in @((Join-Path $agentDir "remote-agent.exe"), (Join-Path $agentDir "target\release\remote-agent.exe"))) {
    if ((-not $Dev) -and (Test-Path $candidate)) { $exe = $candidate; break }
}
if ($exe) {
    Write-Host "[agent] lancement de $exe"
    & $exe @agentArgs
    exit $LASTEXITCODE
}

# --- Sinon : cargo ---
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) { $env:PATH = "$cargoBin;$env:PATH" }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "Ni remote-agent.exe ni Rust (cargo) sur ce PC. Deux solutions :"
    Write-Host "  A. Copier remote-agent.exe (compile sur un autre PC Windows) a cote de ce script : plus rien a installer ici."
    Write-Host "  B. Installer Rust : https://rustup.rs (options par defaut, puis 'Visual Studio Build Tools' avec 'Desktop development with C++'),"
    Write-Host "     fermer et rouvrir le terminal, puis relancer ce script."
    exit 1
}
$cargoArgs = @("run")
if (-not $Dev) { $cargoArgs += "--release" }
$cargoArgs += "--"
$cargoArgs += $agentArgs
Write-Host "[agent] la premiere compilation prend quelques minutes ; ensuite c'est instantane."
Push-Location $agentDir
try { & cargo @cargoArgs } finally { Pop-Location }
