# Windows Code Signing — Azure Artifact Signing (Authoritative)

This document defines the **single, authoritative process** for
Windows Authenticode signing for Quilt Sync.

**Quilt Data, Inc.** is a **10-year-old U.S. company** and uses
**Azure Artifact Signing (Trusted Signing)** exclusively.

## Overview

- Trust model: **Microsoft-managed signing (Public Trust)**
- Key management: **Keys never leave Azure**
- CI authentication: **GitHub → Azure (OIDC)**
- Artifacts that must be signed:
  - `.exe` (NSIS installer)
  - `.msi`

This signing satisfies:

- Windows Authenticode requirements
- Microsoft Defender SmartScreen

Reference:

- <https://azure.microsoft.com/products/artifact-signing>

## 1. Prerequisites

- Azure subscription owned by **Quilt Data, Inc.**
  - 1Password: Quilt -> Shared -> Microsoft SharePoint 365
  - ID: <Ernest.Prabhakar@quiltdata.com>
  - Contact email: <ernest@quilt.bio>
- Azure tenant with long-standing, verifiable history
- GitHub repository with Actions enabled

## 2. Create Azure Artifact Signing Resource

- Create an **Artifact Signing** resource in Azure
  - Subscription: Azure subscription 1
  - Resource group: QuiltSync
  - Account: quilt
- Trust profile: **Public Trust (Windows Authenticode)**
- Publisher identity: **Quilt Data, Inc.**
- Region: US East

Reference:

- <https://learn.microsoft.com/azure/security/trusted-signing/overview>

## 3. Configure GitHub → Azure Authentication

- Use **GitHub OIDC** (required; no client secrets)
- Create a federated credential for the GitHub repository
- Grant the identity permission to the Artifact Signing resource

Reference:

- <https://learn.microsoft.com/azure/developer/github/connect-from-azure>

## 4. Configure CI for Azure Signing

### 4.1 Forbidden legacy configuration

The following **must not exist** in GitHub Secrets:

- `WINDOWS_PFX_BASE64`
- `WINDOWS_PFX_PASSWORD`

PFX-based signing is intentionally disallowed.

### 4.2 Azure signing integration

- Use the Azure Artifact Signing GitHub Action **or** signtool integration
- Sign **all** Windows outputs:
  - `*.exe`
  - `*.msi`

Reference:

- <https://github.com/Azure/artifact-signing-action>

## 5. Run a Signed Release Build

- Push a version tag (e.g. `v0.14.0`)
- Confirm GitHub Actions:
  - Builds Windows installers
  - Signs via Azure Artifact Signing
  - Publishes signed artifacts to GitHub Releases

## 6. One-Time Verification

- Download installer from GitHub Releases
- Right-click → **Properties → Digital Signatures**
- Publisher must show **Microsoft Trusted Signing**

## 7. Reputation & Operational Rules

- Do not rotate:
  - Azure tenant
  - Artifact Signing resource
  - Trust profile

The Azure signing resource is the **long-term SmartScreen reputation anchor**.
Changing identities resets reputation.
