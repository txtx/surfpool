#!/usr/bin/env node

/**
 * Generates and updates OpenBook market constants in overrides.yaml
 *
 * Usage: node generate-markets.js
 *
 * This script fetches AMM v4 pools from the Raydium API and updates
 * the constants.openbook_market.options section in overrides.yaml directly.
 */

const fs = require('fs');
const path = require('path');

const RAYDIUM_API = 'https://api-v3.raydium.io/pools/info/list';
const AMM_V4_PROGRAM = '675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8';
const OVERRIDES_FILE = path.join(__dirname, 'overrides.yaml');

// Number of top pools to include
const TOP_POOLS_COUNT = 100;

// Minimum monthly volume to include (in USD)
const MIN_VOLUME_MONTH = 100000; // $100k

async function fetchPools() {
  const allPools = [];
  let page = 1;
  const pageSize = 500;

  while (true) {
    // Sort by 30d volume descending
    const url = `${RAYDIUM_API}?poolType=standard&poolSortField=volume30d&sortType=desc&pageSize=${pageSize}&page=${page}`;
    const response = await fetch(url);

    if (!response.ok) {
      throw new Error(`Failed to fetch: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();

    if (!data.success || !data.data || !data.data.data) {
      throw new Error('Invalid API response');
    }

    const pools = data.data.data;
    if (pools.length === 0) break;

    allPools.push(...pools);

    // If we got fewer pools than requested, we've reached the end
    if (pools.length < pageSize) break;

    // Safety limit
    if (page >= 10) break;

    page++;
  }

  return allPools;
}

function generateId(symbolA, symbolB) {
  // Normalize WSOL to SOL for the ID
  const normA = symbolA === 'WSOL' ? 'SOL' : symbolA;
  const normB = symbolB === 'WSOL' ? 'SOL' : symbolB;
  return `${normA.toLowerCase()}-${normB.toLowerCase()}`;
}

function generateLabel(symbolA, symbolB) {
  // Normalize WSOL to SOL for display
  const normA = symbolA === 'WSOL' ? 'SOL' : symbolA;
  const normB = symbolB === 'WSOL' ? 'SOL' : symbolB;
  return `${normA}/${normB}`;
}

function formatVolume(volume) {
  if (volume >= 1000000) {
    return `$${(volume / 1000000).toFixed(1)}M`;
  } else if (volume >= 1000) {
    return `$${(volume / 1000).toFixed(0)}K`;
  }
  return `$${volume.toFixed(0)}`;
}

function generateOptionsYaml(pools) {
  const lines = [];

  for (const pool of pools) {
    // Trim whitespace from symbols - some tokens have trailing spaces
    const symbolA = pool.mintA.symbol.trim();
    const symbolB = pool.mintB.symbol.trim();
    const id = generateId(symbolA, symbolB);
    const label = generateLabel(symbolA, symbolB);
    const marketId = pool.marketId;
    const volumeMonth = pool.month?.volume || 0;
    const volumeStr = formatVolume(volumeMonth);

    lines.push(`      - id: "${id}"`);
    lines.push(`        label: "${label}"`);
    lines.push(`        value: "${marketId}"`);
    lines.push(`        description: "${label} AMM v4 pool (30d vol: ${volumeStr})"`);
  }

  return lines.join('\n');
}

async function main() {
  console.log('Fetching AMM v4 pools from Raydium API...');

  const allPools = await fetchPools();
  console.log(`Found ${allPools.length} total standard pools`);

  // Filter for AMM v4 pools with OpenBook markets
  const ammV4Pools = allPools.filter(pool => {
    // Must be AMM v4 program
    if (pool.programId !== AMM_V4_PROGRAM) return false;
    // Must have OpenBook market
    if (!pool.marketId) return false;
    // Must have sufficient monthly volume
    const volumeMonth = pool.month?.volume || 0;
    if (volumeMonth < MIN_VOLUME_MONTH) return false;
    return true;
  });

  console.log(`Found ${ammV4Pools.length} AMM v4 pools with OpenBook markets and sufficient volume`);

  // Sort by monthly volume (highest first)
  ammV4Pools.sort((a, b) => {
    const volA = a.month?.volume || 0;
    const volB = b.month?.volume || 0;
    return volB - volA;
  });

  // Take top pools by volume
  const topPools = ammV4Pools.slice(0, TOP_POOLS_COUNT);
  console.log(`Using top ${topPools.length} pools by 30d volume`);

  // Read existing overrides.yaml
  console.log(`Reading ${OVERRIDES_FILE}...`);
  const yamlContent = fs.readFileSync(OVERRIDES_FILE, 'utf8');

  // Update the YAML file with new constants
  const newYamlContent = updateYamlFile(yamlContent, topPools);

  // Write updated yaml
  fs.writeFileSync(OVERRIDES_FILE, newYamlContent);
  console.log(`\nUpdated ${OVERRIDES_FILE} with ${topPools.length} OpenBook markets`);
}

function generateConstantsSection(pools) {
  const optionsYaml = generateOptionsYaml(pools);
  return `constants:
  openbook_market:
    label: OpenBook Market
    description: Select an OpenBook market for the AMM v4 pool
    options:
${optionsYaml}`;
}

function updateYamlFile(yamlContent, pools) {
  // Find the templates section
  const templatesIndex = yamlContent.indexOf('\ntemplates:');
  if (templatesIndex === -1) {
    throw new Error('Could not find templates section in overrides.yaml');
  }

  // Find the start of the file before any constants (or before templates if no constants)
  const constantsIndex = yamlContent.indexOf('\nconstants:');

  let beforeSection;
  if (constantsIndex !== -1 && constantsIndex < templatesIndex) {
    // There's an existing constants section - take everything before it
    beforeSection = yamlContent.substring(0, constantsIndex);
  } else {
    // No constants section - take everything before templates
    beforeSection = yamlContent.substring(0, templatesIndex);
  }

  // Get the templates section and everything after
  const templatesSection = yamlContent.substring(templatesIndex);

  // Generate the new constants section
  const constantsSection = generateConstantsSection(pools);

  // Combine: before + constants + blank line + templates
  return beforeSection + '\n' + constantsSection + '\n' + templatesSection;
}

main().catch(err => {
  console.error('Error:', err.message);
  process.exit(1);
});
