# Save the user PATH (HKCU\Environment\Path) and write it back byte for byte,
# for the Windows installer smoke (installer-smoke.ps1), which lets the
# release installer change it:
#
#   user-path.ps1 -Save <record>      the value's registry type and raw bytes
#                                     (or that there is none) into <record>
#   user-path.ps1 -Restore <record>   writes them back (or removes the value),
#                                     tells Explorer (WM_SETTINGCHANGE), reads
#                                     the value again and compares type and
#                                     bytes; removes <record> when identical
#
# -Save refuses when <record> is already there: a smoke ended before it wrote
# the PATH back left it, and it holds the PATH from before that smoke. Only
# the user's own value is touched; the system PATH (HKLM) never is.
# Exits 0 on success, 1 otherwise (the record stays).
param(
    [string]$Save = '',
    [string]$Restore = ''
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KlurisUserPath {
  static readonly UIntPtr CurrentUser = new UIntPtr(0x80000001u);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegOpenKeyExW(UIntPtr key, string sub, uint options, int sam, out UIntPtr result);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegQueryValueExW(UIntPtr key, string name, IntPtr reserved, out uint type, byte[] data, ref uint size);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegSetValueExW(UIntPtr key, string name, int reserved, uint type, byte[] data, uint size);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode)] static extern int RegDeleteValueW(UIntPtr key, string name);
  [DllImport("advapi32.dll")] static extern int RegCloseKey(UIntPtr key);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);
  const int KeyRead = 0x20019, KeySetValue = 0x0002;
  static UIntPtr Open(int sam) {
    UIntPtr key;
    int status = RegOpenKeyExW(CurrentUser, "Environment", 0, sam, out key);
    if (status != 0) throw new Exception("RegOpenKeyEx HKCU\\Environment: " + status);
    return key;
  }
  // The raw bytes and type of HKCU\Environment\Path, or null when there is none.
  public static byte[] Read(out uint type) {
    UIntPtr key = Open(KeyRead);
    try {
      uint size = 0;
      int status = RegQueryValueExW(key, "Path", IntPtr.Zero, out type, null, ref size);
      if (status == 2) { type = 0; return null; }
      if (status != 0 && status != 234) throw new Exception("RegQueryValueEx Path: " + status);
      byte[] data = new byte[size];
      status = RegQueryValueExW(key, "Path", IntPtr.Zero, out type, data, ref size);
      if (status != 0) throw new Exception("RegQueryValueEx Path: " + status);
      if (size != data.Length) Array.Resize(ref data, (int)size);
      return data;
    } finally { RegCloseKey(key); }
  }
  // Writes the bytes back with their type, or removes the value (data null).
  public static void Write(uint type, byte[] data) {
    UIntPtr key = Open(KeySetValue);
    try {
      int status = data == null ? RegDeleteValueW(key, "Path") : RegSetValueExW(key, "Path", 0, type, data, (uint)data.Length);
      if (status != 0 && !(data == null && status == 2)) throw new Exception("writing Path: " + status);
    } finally { RegCloseKey(key); }
  }
  public static void Broadcast() {
    UIntPtr result;
    SendMessageTimeoutW(new IntPtr(0xffff), 0x001A, UIntPtr.Zero, "Environment", 0x0002, 5000, out result);
  }
}
'@

function Describe([bool]$present, [uint32]$type, $bytes) {
    if (-not $present) { return 'absent' }
    $name = switch ($type) { 1 { 'REG_SZ' } 2 { 'REG_EXPAND_SZ' } default { "type $type" } }
    return "$($bytes.Length) bytes, $name"
}

try {
    if ($Save -and -not $Restore) {
        if (Test-Path -LiteralPath $Save) {
            Write-Output "user-path: FAIL: $Save is already there: an earlier smoke did not write the user PATH back. Run user-path.ps1 -Restore `"$Save`" first."
            exit 1
        }
        [uint32]$type = 0
        $bytes = [KlurisUserPath]::Read([ref]$type)
        $record = [ordered]@{
            present = ($null -ne $bytes)
            type = $type
            bytes = if ($null -eq $bytes) { '' } else { [Convert]::ToBase64String($bytes) }
        }
        [IO.File]::WriteAllText($Save, ($record | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding($false)))
        Write-Output "user-path: saved the user PATH: $(Describe $record.present $type $bytes)"
        exit 0
    }
    if ($Restore -and -not $Save) {
        $record = [IO.File]::ReadAllText($Restore) | ConvertFrom-Json
        $saved = if ($record.present) { [Convert]::FromBase64String($record.bytes) } else { $null }
        [uint32]$savedType = $record.type
        [KlurisUserPath]::Write($savedType, $saved)
        [KlurisUserPath]::Broadcast()
        [uint32]$backType = 0
        $back = [KlurisUserPath]::Read([ref]$backType)
        $same = ($null -eq $saved) -eq ($null -eq $back)
        if ($same -and $null -ne $saved) {
            $same = ($backType -eq $savedType) -and ($saved.Length -eq $back.Length)
            for ($i = 0; $same -and $i -lt $saved.Length; $i++) { $same = $saved[$i] -eq $back[$i] }
        }
        if (-not $same) {
            Write-Output "user-path: FAIL: the user PATH read back is $(Describe ($null -ne $back) $backType $back), NOT identical to the saved $(Describe ($null -ne $saved) $savedType $saved); the record stays at $Restore"
            exit 1
        }
        Remove-Item -LiteralPath $Restore -Force
        Write-Output "user-path: user PATH restored: $(Describe ($null -ne $back) $backType $back), byte for byte identical to the saved value"
        exit 0
    }
    Write-Output 'usage: user-path.ps1 -Save <record> | -Restore <record>'
    exit 2
} catch {
    Write-Output "user-path: FAIL: $($_.Exception.Message)"
    exit 1
}
