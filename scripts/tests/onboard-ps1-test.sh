#!/bin/sh
# Drives the PowerShell onboarding sign-in against a stub `jiotv` so the
# Windows flow is exercised without a Windows machine or a real account.
#
# The wizard code is sliced out of install.ps1 between the onboarding-ui
# region markers; the interactive prompts are replaced with queued answers.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
installer="$here/../install.ps1"

if [ ! -f "$installer" ]; then
    echo "cannot find $installer" >&2
    exit 1
fi

if ! command -v pwsh >/dev/null 2>&1; then
    if [ -n "${CI:-}" ]; then
        echo "pwsh is required in CI for the PowerShell onboarding test" >&2
        exit 1
    fi
    echo "pwsh is not installed; skipping the PowerShell onboarding test"
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/jiotv-ps1.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

cat > "$tmp/jiotv" <<'STUB'
#!/bin/sh
# Stand-in for the real binary: same prompts, same success lines, a wrong OTP
# fails the way the real one does.
log=$JIO_STUB_LOG
case "$1 $2" in
"login otp")
    printf 'Enter your mobile number: +91 '
    read -r number
    echo "number=$number" >> "$log"
    echo 'Sending OTP to your mobile number'
    echo 'OTP sent to your mobile number'
    printf 'Enter OTP: '
    read -r otp
    echo "otp=$otp" >> "$log"
    if [ "$otp" = 424242 ]; then
        echo 'Login successful'
        exit 0
    fi
    echo 'the OTP is wrong or has expired' >&2
    exit 1
    ;;
"tv+ login"|"extras login")
    echo "cmd=$1" >> "$log"
    printf 'Mobile number registered to the fibre account: +91 '
    read -r number
    echo "number=$number" >> "$log"
    if [ "${JIO_STUB_NO_CONNECTIONS:-0}" = 1 ]; then
        printf 'OTP: '
    else
        echo 'Connections on this number:'
        echo '  1. Home, JioFiber, 1111'
        echo '  2. Office, JioFiber, 2222'
        printf 'Choose a connection: '
        read -r pick
        echo "pick=$pick" >> "$log"
        printf 'OTP: '
    fi
    read -r otp
    echo "otp=$otp" >> "$log"
    if [ "$otp" = 424242 ]; then
        echo 'JioTV+ login saved for Office, JioFiber, 2222'
        exit 0
    fi
    echo 'the OTP is wrong or has expired' >&2
    exit 1
    ;;
"fastway login")
    printf 'Fastway username: '
    read -r user
    echo "user=$user" >> "$log"
    printf 'Fastway password: '
    read -r password
    echo "password=$password" >> "$log"
    if [ "$user" = customer ] && [ "$password" = s3cret ]; then
        echo 'Fastway login saved. Restart the server to use it.'
        exit 0
    fi
    echo 'login failed: bad credentials' >&2
    exit 1
    ;;
*)
    echo "unexpected command: $*" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$tmp/jiotv"

cat > "$tmp/harness.ps1" <<'HARNESS'
param(
    [string]$Installer,
    [string]$Stub,
    [string]$LogPath
)
$ErrorActionPreference = 'Stop'

# Slice the wizard out of the real installer so the test cannot drift from it.
$text = Get-Content -LiteralPath $Installer -Raw
$region = [regex]::Match($text, '(?s)#region onboarding-ui(.*?)#endregion onboarding-ui')
if (-not $region.Success) { throw 'onboarding-ui region not found in install.ps1' }
. ([scriptblock]::Create($region.Groups[1].Value))

# Answer the prompts from variables instead of the console.
$script:answerText = ''
$script:answerSecret = ''
$script:answerPick = 0
$script:pickLabels = ''
$script:retry = $false

function Read-Text([string]$Question, [string]$Default = '') { return $script:answerText }
function Read-Secret([string]$Question) { return $script:answerSecret }
function Select-Option([string]$Question, [object[]]$Options, [int]$Default = 0) {
    if ($Question -like 'Choose the connection*') {
        $script:pickLabels = ($Options | ForEach-Object { $_.Label }) -join ' | '
        return $Options[$script:answerPick].Value
    }
    return $script:retry
}

$env:JIO_STUB_LOG = $LogPath

$script:failures = @()
function Check([string]$Name, [bool]$Condition) {
    if ($Condition) { Write-Host "ok   $Name" }
    else {
        Write-Host "FAIL $Name" -ForegroundColor Red
        $script:failures += $Name
    }
}
function Reset-Log { Set-Content -LiteralPath $LogPath -Value '' -NoNewline }
function Log-Text {
    if (-not (Test-Path -LiteralPath $LogPath)) { return '' }
    return ((Get-Content -LiteralPath $LogPath) -join "`n")
}

# JioTV: the number goes in, the OTP is asked here and goes in next.
Reset-Log
$script:answerText = '9999999999'
$script:answerSecret = '424242'
Check 'JioTV sign-in succeeds' (Connect-JioTv $Stub)
Check 'JioTV sends the number then the OTP' ((Log-Text) -eq "number=9999999999`notp=424242")

Reset-Log
$script:answerSecret = '000000'
Check 'JioTV rejects a wrong OTP' (-not (Connect-JioTv $Stub))

# A stray value on the pipeline would turn the answer into a list and make a
# failed sign-in look successful.
Reset-Log
$script:answerSecret = '424242'
Check 'JioTV returns a single boolean' ((Connect-JioTv $Stub) -is [bool])

# JioTV+: the connection list is read from the command output. The wizard uses
# the command name the binary advertises, so check both spellings.
Reset-Log
$script:TvPlusCmd = 'tv+ login'
$script:answerText = '9999999999'
$script:answerSecret = '424242'
$script:answerPick = 1
Check 'JioTV+ sign-in succeeds' (Connect-Extras $Stub)
Check 'JioTV+ offers the parsed connections' ($script:pickLabels -eq 'Home, JioFiber, 1111 | Office, JioFiber, 2222')
Check 'JioTV+ sends the chosen index' ((Log-Text) -eq "cmd=tv+`nnumber=9999999999`npick=2`notp=424242")

# A number with no selectable connections must not wait for a picker.
Reset-Log
$script:TvPlusCmd = ''
$script:pickLabels = 'unchanged'
$script:answerText = '9999999999'
$script:answerSecret = '424242'
$env:JIO_STUB_NO_CONNECTIONS = '1'
Check 'JioTV+ skips the picker when there is nothing to pick' (Connect-Extras $Stub)
Check 'JioTV+ still signs in without a picker' ((Log-Text) -eq "cmd=extras`nnumber=9999999999`notp=424242")
Check 'JioTV+ does not offer an empty picker' ($script:pickLabels -eq 'unchanged')
Remove-Item Env:JIO_STUB_NO_CONNECTIONS

# Fastway: username and password, no OTP.
Reset-Log
$script:answerText = 'customer'
$script:answerSecret = 's3cret'
Check 'Fastway sign-in succeeds' (Connect-Fastway $Stub)
Check 'Fastway sends both credentials' ((Log-Text) -eq "user=customer`npassword=s3cret")

# A failed attempt is offered a retry, and saying no ends the sign-in.
Reset-Log
$script:answerText = '9999999999'
$script:answerSecret = '000000'
$script:retry = $false
Check 'declining a retry ends the sign-in' (-not (Connect-Provider $Stub 'jiotv'))
Check 'a refused attempt is not repeated' ((Log-Text) -eq "number=9999999999`notp=000000")

if ($script:failures.Count -gt 0) {
    Write-Host ''
    Write-Host "$($script:failures.Count) check(s) failed" -ForegroundColor Red
    exit 1
}
exit 0
HARNESS

if pwsh -NoProfile -NonInteractive -File "$tmp/harness.ps1" \
    -Installer "$installer" -Stub "$tmp/jiotv" -LogPath "$tmp/calls.log"; then
    echo "onboarding (PowerShell) OK"
else
    echo "onboarding (PowerShell) FAILED" >&2
    exit 1
fi
