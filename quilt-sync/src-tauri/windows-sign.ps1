# Signs one file with Azure Trusted Signing. Tauri runs it for every file it
# signs (app .exe, .msi) in the Windows release job, before the updater
# signature is made and before tauri-action uploads. See windows-sign.conf.json
# and docs/windows-signing.md.
param([Parameter(Mandatory)][string]$File)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

# GitHub's OIDC token lives only minutes and Tauri signs at the end of a long
# build, so fetch a fresh one and log in to Azure for each file.
$oidc = Invoke-RestMethod `
  -Headers @{ Authorization = "Bearer $env:ACTIONS_ID_TOKEN_REQUEST_TOKEN" } `
  -Uri "$env:ACTIONS_ID_TOKEN_REQUEST_URL&audience=api://AzureADTokenExchange"
az login --service-principal --output none --allow-no-subscriptions `
  --username $env:AZURE_CLIENT_ID `
  --tenant $env:AZURE_TENANT_ID `
  --federated-token $oidc.value

$params = @{
  Endpoint                            = $env:AZURE_TRUSTED_SIGNING_ENDPOINT
  CodeSigningAccountName              = $env:AZURE_TRUSTED_SIGNING_ACCOUNT
  CertificateProfileName              = $env:AZURE_TRUSTED_SIGNING_PROFILE
  Files                               = $File
  Description                         = 'QuiltSync'
  FileDigest                          = 'SHA256'
  TimestampRfc3161                    = 'http://timestamp.acs.microsoft.com'
  TimestampDigest                     = 'SHA256'
  # Use only the `az login` above.
  ExcludeEnvironmentCredential        = $true
  ExcludeWorkloadIdentityCredential   = $true
  ExcludeManagedIdentityCredential    = $true
  ExcludeSharedTokenCacheCredential   = $true
  ExcludeVisualStudioCredential       = $true
  ExcludeVisualStudioCodeCredential   = $true
  ExcludeAzurePowerShellCredential    = $true
  ExcludeAzureDeveloperCliCredential  = $true
  ExcludeInteractiveBrowserCredential = $true
}
Invoke-ArtifactSigning @params

$status = (Get-AuthenticodeSignature -LiteralPath $File).Status
if ($status -ne 'Valid') {
  throw "Signing $File failed: signature status is $status"
}
