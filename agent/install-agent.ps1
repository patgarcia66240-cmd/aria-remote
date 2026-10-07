# install-agent.ps1 - installe (ou met a jour) l'agent Remote sur le PC A CONTROLER, sans compiler ni copier de fichier.
#
# Ce que fait le script :
#   1. cherche la derniere version publiee de l'agent sur GitHub (ou celle demandee avec -Version) ;
#   2. telecharge remote-agent.exe et VERIFIE son empreinte SHA-256 ;
#   3. l'installe dans %LOCALAPPDATA%\ARIA Remote (aucun droit administrateur) ;
#   4. cree un raccourci dans le menu Demarrer (et sur le Bureau avec -Desktop, au demarrage de Windows avec -Autostart) ;
#   5. lance l'agent : sa fenetre demande l'adresse du serveur et sa cle au premier lancement, puis les retient.
#
# Rien ne tourne en arriere-plan : l'agent n'est actif que tant que sa fenetre est ouverte.
#
# Pas d'emoji ni d'accent dans ce fichier volontairement : Windows PowerShell 5.1 lit parfois un .ps1 avec l'encodage de la console
# plutot que l'UTF-8 reel du fichier (meme precaution que start-agent.ps1).
#
# Usage :
#   .\install-agent.ps1                                   # derniere version, puis lancement
#   .\install-agent.ps1 -Server https://rendezvous.exemple.fr -ApiKey <cle>   # pre-remplit l'adresse et la cle de l'agent
#   .\install-agent.ps1 -Name "PC du salon" -Allow "mouse,keyboard"
#   .\install-agent.ps1 -Version 1.5.0                    # une version precise
#   .\install-agent.ps1 -Desktop -Autostart               # raccourci sur le Bureau et lancement a l'ouverture de session
#   .\install-agent.ps1 -NoLaunch                         # installe sans lancer
#   .\install-agent.ps1 -Check                            # indique seulement la derniere version, sans rien installer
#   .\install-agent.ps1 -Uninstall                        # retire le programme et les raccourcis (garde l'identite de l'appareil)
#   .\install-agent.ps1 -Uninstall -Purge                 # retire AUSSI l'identite de l'appareil (il faudra le reappairer)
#
# Une seule ligne depuis PowerShell (sans telecharger le fichier) :
#   irm https://raw.githubusercontent.com/patgarcia66240-cmd/aria-remote/main/agent/install-agent.ps1 | iex
# avec des options :
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/patgarcia66240-cmd/aria-remote/main/agent/install-agent.ps1))) -Desktop -Autostart

param(
    [string]$Version = "",
    [string]$Server = "",
    [string]$ApiKey = "",
    [string]$Name = "",
    [string]$Allow = "",
    [string]$Repo = "patgarcia66240-cmd/aria-remote",
    [switch]$Desktop,
    [switch]$Autostart,
    [switch]$NoLaunch,
    [switch]$Check,
    [switch]$Uninstall,
    [switch]$Purge
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"      # Invoke-WebRequest est beaucoup plus rapide sans barre de progression (PowerShell 5.1)
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }

$AppName = "ARIA Remote"
$ExeName = "remote-agent.exe"

function Get-InstallDir { Join-Path $env:LOCALAPPDATA $AppName }
function Get-ConfigDir { Join-Path $env:APPDATA "pc-assistant-remote-agent" }
function Get-StartMenuLink { Join-Path ([Environment]::GetFolderPath("Programs")) "$AppName.lnk" }
function Get-DesktopLink { Join-Path ([Environment]::GetFolderPath("Desktop")) "$AppName.lnk" }
function Get-StartupLink { Join-Path ([Environment]::GetFolderPath("Startup")) "$AppName.lnk" }

# Choisit la release a installer : la plus recente (hors brouillon) dont l'etiquette est agent-v<version>, ou celle demandee.
function Select-AgentRelease {
    param($Releases, [string]$Wanted)
    $agent = @($Releases | Where-Object { (-not $_.draft) -and ($_.tag_name -like "agent-v*") })
    if ($Wanted) {
        $tag = "agent-v" + $Wanted.TrimStart("v")
        $found = @($agent | Where-Object { $_.tag_name -eq $tag })
        if ($found.Count -eq 0) { throw "La version $Wanted n'existe pas. Versions publiees : " + (($agent | ForEach-Object { $_.tag_name.Substring(7) }) -join ", ") }
        return $found[0]
    }
    if ($agent.Count -eq 0) { throw "Aucune version de l'agent n'est publiee dans $Repo." }
    return ($agent | Sort-Object { [datetime]$_.published_at } -Descending | Select-Object -First 1)
}

# Premier mot d'un fichier .sha256 ("<empreinte>  remote-agent.exe").
function Read-Sha256File {
    param([string]$Text)
    $first = ($Text.Trim() -split "\s+")[0].ToLower()
    if ($first -notmatch "^[0-9a-f]{64}$") { throw "Fichier d'empreinte illisible." }
    return $first
}

function Test-Sha256 {
    param([string]$Path, [string]$Expected)
    $actual = (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $Expected.ToLower()) { throw "Empreinte SHA-256 differente : le fichier est corrompu ou a ete modifie ($actual au lieu de $Expected). Rien n'a ete installe." }
}

function New-Shortcut {
    param([string]$Link, [string]$Target, [string]$Arguments = "")
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($Link)
    $shortcut.TargetPath = $Target
    $shortcut.Arguments = $Arguments
    $shortcut.WorkingDirectory = Split-Path -Parent $Target
    $shortcut.Description = "Agent Remote d'ARIA : permet a ARIA de se connecter a ce PC, avec ton accord"
    $shortcut.Save()
}

function Stop-Agent {
    # Une version en cours d'execution verrouille le fichier : on la ferme avant de la remplacer.
    $running = @(Get-Process -Name "remote-agent" -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        Write-Host "[agent] fermeture de l'agent en cours d'execution..."
        $running | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
}

# Permet de charger les fonctions sans rien executer (tests) : . .\install-agent.ps1
if ($MyInvocation.InvocationName -eq ".") { return }

# --- Desinstallation ---------------------------------------------------------------------------------------------------------------
if ($Uninstall) {
    Stop-Agent
    foreach ($link in @((Get-StartMenuLink), (Get-DesktopLink), (Get-StartupLink))) { if (Test-Path $link) { Remove-Item $link -Force } }
    $dir = Get-InstallDir
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
    Write-Host "[agent] programme et raccourcis retires."
    if ($Purge) {
        if (Test-Path (Get-ConfigDir)) { Remove-Item (Get-ConfigDir) -Recurse -Force }
        Write-Host "[agent] identite de l'appareil supprimee : il faudra le reappairer."
    } else {
        Write-Host "[agent] identite de l'appareil conservee ($(Get-ConfigDir)). Ajoute -Purge pour la supprimer aussi."
    }
    exit 0
}

# --- Recherche de la version -------------------------------------------------------------------------------------------------------
Write-Host "[agent] recherche de la version a installer ($Repo)..."
$headers = @{ "User-Agent" = "aria-remote-installer"; "Accept" = "application/vnd.github+json" }
try {
    $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=50" -Headers $headers
} catch {
    throw "Impossible de joindre GitHub ($($_.Exception.Message)). Verifie la connexion Internet."
}
$release = Select-AgentRelease -Releases $releases -Wanted $Version
$number = $release.tag_name.Substring(7)
Write-Host "[agent] version $number (publiee le $(([datetime]$release.published_at).ToString('yyyy-MM-dd')))"
if ($Check) { exit 0 }

$exeAsset = @($release.assets | Where-Object { $_.name -eq $ExeName })[0]
$sumAsset = @($release.assets | Where-Object { $_.name -eq "$ExeName.sha256" })[0]
if ((-not $exeAsset) -or (-not $sumAsset)) { throw "La version $number ne contient pas $ExeName et son empreinte." }

# --- Telechargement et verification (dans un dossier temporaire : l'installation existante n'est touchee qu'apres) ---------------------
$temp = Join-Path ([IO.Path]::GetTempPath()) ("aria-remote-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $tempExe = Join-Path $temp $ExeName
    Write-Host "[agent] telechargement..."
    Invoke-WebRequest -Uri $exeAsset.browser_download_url -OutFile $tempExe -UseBasicParsing -Headers @{ "User-Agent" = "aria-remote-installer" }
    $expected = Read-Sha256File -Text ((Invoke-WebRequest -Uri $sumAsset.browser_download_url -UseBasicParsing -Headers @{ "User-Agent" = "aria-remote-installer" }).Content | Out-String)
    Test-Sha256 -Path $tempExe -Expected $expected
    Write-Host "[agent] empreinte verifiee : $expected"

    # --- Installation ----------------------------------------------------------------------------------------------------------
    $dir = Get-InstallDir
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Stop-Agent
    $exe = Join-Path $dir $ExeName
    Copy-Item -Path $tempExe -Destination $exe -Force
    Unblock-File -Path $exe -ErrorAction SilentlyContinue     # retire la marque "telecharge depuis Internet" : pas d'avertissement a chaque lancement
} finally {
    Remove-Item -Path $temp -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Raccourcis ------------------------------------------------------------------------------------------------------------------
New-Shortcut -Link (Get-StartMenuLink) -Target $exe
Write-Host "[agent] raccourci : menu Demarrer > $AppName"
if ($Desktop) { New-Shortcut -Link (Get-DesktopLink) -Target $exe; Write-Host "[agent] raccourci sur le Bureau" }
if ($Autostart) { New-Shortcut -Link (Get-StartupLink) -Target $exe; Write-Host "[agent] l'agent s'ouvrira a l'ouverture de session Windows" }
elseif (Test-Path (Get-StartupLink)) { Remove-Item (Get-StartupLink) -Force }

Write-Host ""
Write-Host "[agent] installe dans $dir"
if ($NoLaunch) { Write-Host "[agent] lance-le depuis le menu Demarrer quand tu veux."; exit 0 }

# --- Lancement : seulement ce qui est explicite, pour ne jamais ecraser l'adresse deja retenue ---------------------------------------
$agentArgs = @()
if ($PSBoundParameters.ContainsKey("Allow")) { $agentArgs += @("--allow", $Allow) }
if ($Server) { $agentArgs += @("--server", $Server) }
if ($ApiKey) { $agentArgs += @("--api-key", $ApiKey) }
if ($Name) { $agentArgs += @("--name", $Name) }
Write-Host "[agent] lancement : la fenetre de l'agent affiche le code d'appairage."
Start-Process -FilePath $exe -ArgumentList $agentArgs -WorkingDirectory $dir
