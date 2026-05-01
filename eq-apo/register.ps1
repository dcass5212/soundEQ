# eq-apo/register.ps1
#
# Registers the SoundEQ APO DLL as a Windows audio system effect and
# installs it on all active audio render endpoints (speakers, headphones).
#
# Run as Administrator. After registering, restart the Windows Audio service
# or reboot to activate the APO.
#
# Usage:
#   .\register.ps1             # register + install on all endpoints
#   .\register.ps1 -Unregister # remove from all endpoints and delete registry keys
#
# What this script does:
#   1. Registers the COM class in HKLM\Software\Classes\CLSID\{our-guid}
#   2. Adds an MFX entry under each audio endpoint's FxProperties key so
#      Windows includes our APO in the audio pipeline for that device.
#
# APO type: MFX (Mode Effects)
#   MFX runs after endpoint volume is applied. This means system volume
#   knobs, per-app volume mixers, and the Windows volume slider all work
#   correctly when SoundEQ is installed as an MFX APO.
#   LFX (Local Effects) runs per-stream, before mixing — we don't want that.
#   GFX (Global Effects) runs globally but before endpoint volume — also wrong.
#
# Why we need privilege escalation:
#   On Windows 10/11, the FxProperties key under each MMDevices endpoint is
#   owned by SYSTEM and its DACL denies write access to Administrators.
#   We use SeTakeOwnershipPrivilege (present in every admin token but not
#   enabled by default) to take ownership, then grant ourselves FullControl
#   before writing the MFX CLSID value.

param(
    [switch]$Unregister,

    # -TestSign: Create a self-signed certificate, install it as machine-trusted,
    # and sign the DLL before registering. No reboot needed.
    # APO DLLs are user-mode — bcdedit test-signing mode is for kernel drivers only.
    # Do NOT use in production — get a CA-issued code-signing certificate for that.
    [switch]$TestSign
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Configuration ─────────────────────────────────────────────────────────────

$CLSID        = "{8C2A5F3E-B47D-4A1C-9E8F-D0C3B6A2E1F4}"
$FriendlyName = "SoundEQ Parametric EQ"

# Source DLL — the compiled output from `cargo build --release -p eq-apo`.
$DllSource = Join-Path $PSScriptRoot "..\target\release\eq_apo.dll"
$DllSource = [System.IO.Path]::GetFullPath($DllSource)

# Install path — C:\Users\Public is readable by all service accounts including
# LOCAL SERVICE (audiodg.exe). The project directory (C:\soundEQ\...) may deny
# traverse access to service accounts, so we copy here before registering.
$InstallDir = "C:\Users\Public\soundEQ"
$DllPath    = Join-Path $InstallDir "eq_apo.dll"

# PKEY_FX_ModeEffectClsid — the MFX (mode effect) slot, index 4.
# Procmon confirmed audiodg.exe reads index 10 (MFX supported-modes list)
# for the active endpoint. Index 10 must be present alongside the CLSID at
# index 4 or the audio engine skips loading the APO entirely.
# MFX runs after mixing and after endpoint volume — system volume works.
#
# Index 10 is PKEY_FX_ProcessingModes_Supported_For_Mode_Effects — a
# REG_MULTI_SZ list of audio processing mode GUIDs. We advertise only
# AUDIO_SIGNALPROCESSINGMODE_DEFAULT ({C18E2F7E-...}) since we apply EQ
# to all audio regardless of the signal processing mode.
$MfxClsidKey = "{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},4"
$MfxModesKey = "{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},10"
$DefaultModeGuid = "{C18E2F7E-933D-4965-B7D1-1EEF228D2AF3}"

# Legacy slots written during earlier attempts — cleaned up on unregister.
$LegacyPreMixKey = "{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},0"
$LegacyPreMix9Key = "{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},9"

# ── Privilege helpers ─────────────────────────────────────────────────────────
# P/Invoke shim that calls AdjustTokenPrivileges to enable a named privilege
# on the current process token. PowerShell's built-in cmdlets cannot do this.

$PrivilegeDef = @"
using System;
using System.Runtime.InteropServices;
public class WinPrivilege {
    [DllImport("advapi32.dll", ExactSpelling = true, SetLastError = true)]
    static extern bool AdjustTokenPrivileges(IntPtr htok, bool disall,
        ref TokPriv1Luid newst, int len, IntPtr prev, IntPtr relen);
    [DllImport("advapi32.dll", ExactSpelling = true, SetLastError = true)]
    static extern bool OpenProcessToken(IntPtr h, int acc, ref IntPtr phtok);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool LookupPrivilegeValue(string host, string name, ref long pluid);
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct TokPriv1Luid { public int Count; public long Luid; public int Attr; }
    const int SE_PRIVILEGE_ENABLED  = 2;
    const int TOKEN_QUERY           = 8;
    const int TOKEN_ADJUST_PRIVS    = 0x20;
    public static bool Enable(IntPtr processHandle, string privilege) {
        var tp = new TokPriv1Luid { Count = 1, Luid = 0, Attr = SE_PRIVILEGE_ENABLED };
        IntPtr htok = IntPtr.Zero;
        OpenProcessToken(processHandle, TOKEN_ADJUST_PRIVS | TOKEN_QUERY, ref htok);
        LookupPrivilegeValue(null, privilege, ref tp.Luid);
        return AdjustTokenPrivileges(htok, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero);
    }
}
"@
Add-Type -TypeDefinition $PrivilegeDef

function Enable-Privilege([string]$Name) {
    $handle = [System.Diagnostics.Process]::GetCurrentProcess().Handle
    [WinPrivilege]::Enable($handle, $Name) | Out-Null
}

# Take ownership of a registry key and grant Administrators FullControl.
# $PsPath is a PowerShell registry path like "HKLM:\SOFTWARE\...".
# Called on both the endpoint key (so New-Item on FxProperties succeeds)
# and on FxProperties itself (so Set-ItemProperty succeeds).
function Grant-AdminWrite([string]$PsPath) {
    # Strip the PowerShell drive/provider prefix — Win32 APIs want a plain subkey
    # path. Get-ChildItem returns full PSPaths like:
    #   Microsoft.PowerShell.Core\Registry::HKEY_LOCAL_MACHINE\SOFTWARE\...
    # or the short form HKLM:\SOFTWARE\... — handle both.
    $subKey = $PsPath -replace '^.*HKEY_LOCAL_MACHINE\\', ''

    # SeTakeOwnershipPrivilege: lets us take ownership of any securable object
    # regardless of its current DACL. SeRestorePrivilege: lets us set the owner
    # to an arbitrary principal (not just ourselves).
    Enable-Privilege "SeTakeOwnershipPrivilege"
    Enable-Privilege "SeRestorePrivilege"

    # Step 1: open with TakeOwnership rights and set owner to Administrators.
    $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey(
        $subKey,
        [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
        [System.Security.AccessControl.RegistryRights]::TakeOwnership
    )
    if ($null -eq $key) { throw "Grant-AdminWrite: could not open key: $subKey" }

    $acl = $key.GetAccessControl([System.Security.AccessControl.AccessControlSections]::None)
    $acl.SetOwner([System.Security.Principal.NTAccount]"Administrators")
    $key.SetAccessControl($acl)
    $key.Close()

    # Step 2: re-open with ChangePermissions rights (now that we own it) and
    # explicitly set ACEs for SYSTEM, LOCAL SERVICE, and Administrators.
    #
    # Why we must be explicit: the original FxProperties key was owned by SYSTEM
    # with no explicit SYSTEM ACE — SYSTEM's access came from being the owner.
    # After we changed the owner to Administrators, SYSTEM lost that implicit
    # access. audiodg.exe (which runs as LOCAL SERVICE) reads FxProperties to
    # discover which APO CLSIDs to load — if it can't read the key, it silently
    # skips the APO and our DLL is never loaded.
    $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey(
        $subKey,
        [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,
        [System.Security.AccessControl.RegistryRights]::ChangePermissions
    )
    $acl = $key.GetAccessControl()

    foreach ($identity in @("NT AUTHORITY\SYSTEM", "BUILTIN\Administrators")) {
        $rule = New-Object System.Security.AccessControl.RegistryAccessRule(
            $identity, "FullControl",
            [System.Security.AccessControl.InheritanceFlags]::None,
            [System.Security.AccessControl.PropagationFlags]::None,
            [System.Security.AccessControl.AccessControlType]::Allow
        )
        $acl.SetAccessRule($rule)
    }

    # audiodg.exe identity — grant read so it can find our CLSID in FxProperties.
    $readRule = New-Object System.Security.AccessControl.RegistryAccessRule(
        "NT AUTHORITY\LOCAL SERVICE", "ReadKey",
        [System.Security.AccessControl.InheritanceFlags]::None,
        [System.Security.AccessControl.PropagationFlags]::None,
        [System.Security.AccessControl.AccessControlType]::Allow
    )
    $acl.SetAccessRule($readRule)

    $key.SetAccessControl($acl)
    $key.Close()
}

# ── Test-signing helpers ──────────────────────────────────────────────────────
#
# Windows 10/11 requires audio APO DLLs to be Authenticode-signed before
# audiodg.exe will load them. Without a signature the DLL is silently rejected
# before DllMain is even called, which is why dllmain.log never appears.
#
# Test signing bypasses the requirement for a CA-issued certificate.
# Use only for local debugging — for production, get a real code-signing cert.

function Enable-TestSigning([string]$DllToSign) {
    # APO DLLs are user-mode COM DLLs, not kernel drivers.
    # bcdedit /set testsigning on only affects kernel-mode driver loading — not needed here.
    # A self-signed cert installed in the machine's trusted root store is enough for
    # audiodg.exe to accept our DLL. No reboot required.
    #
    # If signing still doesn't work, Memory Integrity (HVCI) may be active.
    # Check: Windows Security -> Device Security -> Core isolation -> Memory integrity.
    # If on, a CA-issued cert is required (self-signed will be rejected).

    # Check HVCI status — it blocks self-signed certs in protected user-mode processes.
    $hvciOn = $false
    $dgInfo = Get-CimInstance -ClassName Win32_DeviceGuard -Namespace root\Microsoft\Windows\DeviceGuard -ErrorAction SilentlyContinue
    if ($null -ne $dgInfo) {
        # SecurityServicesRunning is a UInt32[] where each entry is a running service ID.
        # Value 2 = Hypervisor-Protected Code Integrity (HVCI / Memory Integrity).
        $hvciOn = $dgInfo.SecurityServicesRunning -contains 2
    }
    if ($hvciOn) {
        Write-Warning "Memory Integrity (HVCI) is active. Self-signed certs will be rejected."
        Write-Warning "Disable it in Windows Security -> Device Security -> Core isolation,"
        Write-Warning "or purchase a CA-issued code-signing cert (DigiCert, Sectigo, etc.)."
        Write-Warning "Proceeding anyway so the cert is ready if you disable HVCI."
    } else {
        Write-Host "Memory Integrity (HVCI): not active. Self-signed cert should work."
    }

    Write-Host "Creating self-signed code-signing certificate..."
    # Use splatting to avoid backtick line continuations, which are fragile in PS 5.1.
    $certArgs = @{
        Subject           = "CN=SoundEQ Test Signing, O=SoundEQ Dev"
        Type              = "CodeSigningCert"
        KeyUsage          = "DigitalSignature"
        CertStoreLocation = "Cert:\LocalMachine\My"
        NotAfter          = (Get-Date).AddYears(5)
    }
    $cert = New-SelfSignedCertificate @certArgs
    Write-Host "  Thumbprint: $($cert.Thumbprint)"

    # The cert must be in both Root (chain trust) and TrustedPublisher (publisher trust)
    # for audiodg.exe's WinVerifyTrust check to pass. No reboot needed.
    foreach ($storeName in @("Root", "TrustedPublisher")) {
        $storeLocation = [System.Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine
        $store = New-Object System.Security.Cryptography.X509Certificates.X509Store($storeName, $storeLocation)
        $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
        $store.Add($cert)
        $store.Close()
        Write-Host "  Installed in LocalMachine\$storeName."
    }

    Write-Host "Signing DLL: $DllToSign"
    $sig = Set-AuthenticodeSignature -FilePath $DllToSign -Certificate $cert
    if ($sig.Status -eq "Valid") {
        Write-Host "  Signed successfully."
    } else {
        Write-Warning "  Sign result: $($sig.Status) - $($sig.StatusMessage)"
    }
    Write-Host ""
}

# ── General helpers ───────────────────────────────────────────────────────────

function Require-Admin {
    $identity  = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]$identity
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Write-Error "This script must be run as Administrator."
        exit 1
    }
}

function Get-RenderEndpoints {
    # Each subkey under this path is a render endpoint (speakers, headphones, etc.).
    $baseKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render"
    if (-not (Test-Path $baseKey)) { return @() }
    return Get-ChildItem $baseKey | ForEach-Object { $_.PSPath }
}

# ── Register ──────────────────────────────────────────────────────────────────

function Register-Apo {
    if (-not (Test-Path $DllSource)) {
        Write-Error "DLL not found at: $DllSource`nBuild with: cargo build --release -p eq-apo"
        exit 1
    }

    # Copy the DLL to C:\Users\Public\soundEQ\ so audiodg.exe (LOCAL SERVICE)
    # can reach it. The project directory may deny service account traverse access.
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    Copy-Item -Path $DllSource -Destination $DllPath -Force
    Write-Host "Installed DLL to: $DllPath"

    # Sign the DLL if -TestSign was requested.
    # audiodg.exe rejects unsigned APO DLLs silently (DllMain is never called).
    # Test signing is the fastest way to confirm whether signing is the blocker.
    if ($TestSign) { Enable-TestSigning -DllToSign $DllPath }

    # Report current signature status so the user can verify before continuing.
    $sigStatus = (Get-AuthenticodeSignature $DllPath).Status
    Write-Host "DLL signature status: $sigStatus"
    if ($sigStatus -ne "Valid") {
        Write-Warning "DLL is $sigStatus. audiodg.exe will silently refuse to load it."
        Write-Warning "Either run with -TestSign (plus bcdedit /set testsigning on + reboot)"
        Write-Warning "or sign with a real code-signing certificate before testing."
    }

    Write-Host "Registering COM class $CLSID ..."

    # 1. COM class registration under HKLM\Software\Classes\CLSID.
    #    Admin has write access here by default — no privilege tricks needed.
    $clsidKey  = "HKLM:\SOFTWARE\Classes\CLSID\$CLSID"
    New-Item    -Path $clsidKey -Force | Out-Null
    Set-ItemProperty -Path $clsidKey -Name "(Default)" -Value $FriendlyName

    $inprocKey = "$clsidKey\InProcServer32"
    New-Item    -Path $inprocKey -Force | Out-Null
    Set-ItemProperty -Path $inprocKey -Name "(Default)"      -Value $DllPath
    Set-ItemProperty -Path $inprocKey -Name "ThreadingModel" -Value "Both"

    New-Item -Path "$clsidKey\Properties"     -Force | Out-Null
    New-Item -Path "$clsidKey\FX\Association" -Force | Out-Null

    Write-Host "COM class registered."

    # 2. Install on each render endpoint as MFX (index 4 CLSID + index 10 modes).
    #    FxProperties is owned by SYSTEM on Win10/11 — take ownership first.
    $endpoints = Get-RenderEndpoints
    if ($endpoints.Count -eq 0) {
        Write-Warning "No render endpoints found. APO registered but not installed on any device."
        return
    }

    foreach ($ep in $endpoints) {
        $fxKey = "$ep\FxProperties"

        # Grant write on the endpoint key so New-Item can create FxProperties.
        Grant-AdminWrite -PsPath $ep

        if (-not (Test-Path $fxKey)) {
            New-Item -Path $fxKey -Force | Out-Null
        }

        # Grant write on FxProperties itself so Set-ItemProperty succeeds.
        Grant-AdminWrite -PsPath $fxKey

        # Index 4: MFX CLSID — the mode-effect slot (runs after endpoint volume).
        Set-ItemProperty -Path $fxKey -Name $MfxClsidKey -Value $CLSID -Type String

        # Index 10: supported processing modes — required for audiodg.exe to
        # activate the MFX APO. Without this key the engine skips our CLSID.
        Set-ItemProperty -Path $fxKey -Name $MfxModesKey -Value @($DefaultModeGuid) -Type MultiString

        # Remove stale LFX (index 0) and index 9 entries from earlier attempts.
        Remove-ItemProperty -Path $fxKey -Name $LegacyPreMixKey  -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $fxKey -Name $LegacyPreMix9Key -ErrorAction SilentlyContinue

        # Device friendly name is stored as a PKEY-formatted property name, not
        # a plain "DeviceDesc" string — fall back to the endpoint GUID on miss.
        $deviceName = try {
            $props = Get-ItemProperty -Path $ep -ErrorAction Stop
            $props."{a45c254e-df1c-4efd-8020-67d146a850e0},2"
        } catch { $ep -replace '.*\{', '{' }
        Write-Host "  Installed on: $deviceName"
    }

    Write-Host ""
    Write-Host "Done. Restart the Windows Audio service to activate:"
    Write-Host "  net stop audiosrv; net start audiosrv"
    Write-Host "Or reboot."
}

# ── Unregister ────────────────────────────────────────────────────────────────

function Unregister-Apo {
    Write-Host "Removing SoundEQ APO from all render endpoints ..."

    foreach ($ep in (Get-RenderEndpoints)) {
        $fxKey = "$ep\FxProperties"
        if (Test-Path $fxKey) {
            Grant-AdminWrite -PsPath $fxKey
            $mfxVal = try {
                (Get-ItemProperty -Path $fxKey -Name $MfxClsidKey -ErrorAction Stop).$MfxClsidKey
            } catch { $null }
            # Also check legacy slots (index 0 and 9) from previous registration attempts.
            $legacyVal0 = try {
                (Get-ItemProperty -Path $fxKey -Name $LegacyPreMixKey -ErrorAction Stop).$LegacyPreMixKey
            } catch { $null }
            $legacyVal9 = try {
                (Get-ItemProperty -Path $fxKey -Name $LegacyPreMix9Key -ErrorAction Stop).$LegacyPreMix9Key
            } catch { $null }
            if ($mfxVal -eq $CLSID -or $legacyVal0 -eq $CLSID -or $legacyVal9 -eq $CLSID) {
                Remove-ItemProperty -Path $fxKey -Name $MfxClsidKey      -ErrorAction SilentlyContinue
                Remove-ItemProperty -Path $fxKey -Name $MfxModesKey      -ErrorAction SilentlyContinue
                Remove-ItemProperty -Path $fxKey -Name $LegacyPreMixKey  -ErrorAction SilentlyContinue
                Remove-ItemProperty -Path $fxKey -Name $LegacyPreMix9Key -ErrorAction SilentlyContinue
                $deviceName = try {
                    $props = Get-ItemProperty -Path $ep -ErrorAction Stop
                    $props."{a45c254e-df1c-4efd-8020-67d146a850e0},2"
                } catch { $ep -replace '.*\{', '{' }
                Write-Host "  Removed from: $deviceName"
            }
        }
    }

    Write-Host "Removing COM class registration ..."
    $clsidKey = "HKLM:\SOFTWARE\Classes\CLSID\$CLSID"
    if (Test-Path $clsidKey) {
        Remove-Item -Path $clsidKey -Recurse -Force
        Write-Host "  COM class removed."
    }

    Write-Host ""
    Write-Host "Unregistered. Restart Windows Audio to complete removal:"
    Write-Host "  net stop audiosrv; net start audiosrv"
}

# ── Main ──────────────────────────────────────────────────────────────────────

Require-Admin

if ($Unregister) { Unregister-Apo } else { Register-Apo }
