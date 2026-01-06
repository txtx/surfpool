#!/usr/bin/env node

/**
 * Generates and updates Pyth price feed constants in overrides.yaml
 *
 * Usage: node generate-price-feeds.js
 *
 * This script fetches price feeds from the Pyth Hermes API and updates
 * the constants.price_feed.options section in overrides.yaml directly.
 */

const fs = require('fs');
const path = require('path');

const HERMES_API = 'https://hermes.pyth.network/v2/price_feeds';
const HERMES_PRICE_API = 'https://hermes.pyth.network/v2/updates/price/latest';
const OVERRIDES_FILE = path.join(__dirname, 'overrides.yaml');

// Filter for popular Solana ecosystem tokens
const POPULAR_SYMBOLS = new Set([
  // Major cryptos
  'SOL', 'BTC', 'ETH', 'USDC', 'USDT',
  // Solana ecosystem
  'BONK', 'JTO', 'PYTH', 'RAY', 'JUP', 'WIF', 'ORCA', 'MSOL', 'JITOSOL',
  'HNT', 'RENDER', 'MOBILE', 'IOT',
  // Popular alts
  'LINK', 'ARB', 'SUI', 'AVAX', 'DOGE', 'PEPE', 'SHIB',
  'APT', 'OP', 'MATIC', 'ATOM', 'DOT', 'ADA', 'XRP', 'BNB',
  'UNI', 'AAVE', 'MKR', 'CRV', 'LDO', 'GMX', 'PENDLE',
  // Stablecoins
  'DAI', 'FRAX', 'LUSD', 'USDD',
  // Wrapped assets
  'WBTC', 'WETH', 'WSOL',
]);

async function fetchPriceFeeds() {
  const response = await fetch(HERMES_API);
  if (!response.ok) {
    throw new Error(`Failed to fetch: ${response.status} ${response.statusText}`);
  }
  return response.json();
}

async function fetchExponents(feedIds) {
  const exponents = {};
  const batchSize = 50;

  for (let i = 0; i < feedIds.length; i += batchSize) {
    const batch = feedIds.slice(i, i + batchSize);
    const params = batch.map(id => `ids[]=${id}`).join('&');
    const url = `${HERMES_PRICE_API}?${params}&parsed=true`;

    try {
      const response = await fetch(url);
      if (response.ok) {
        const data = await response.json();
        if (data.parsed) {
          for (const item of data.parsed) {
            exponents[item.id] = item.price.expo;
          }
        }
      }
    } catch (e) {
      console.error(`Warning: Failed to fetch exponents for batch ${i}: ${e.message}`);
    }
  }

  return exponents;
}

function formatFeedId(id) {
  return id.startsWith('0x') ? id : `0x${id}`;
}

function generateId(base, quote) {
  return `${base.toLowerCase()}-${quote.toLowerCase()}`;
}

function generateLabel(base, quote) {
  return `${base.toUpperCase()}/${quote.toUpperCase()}`;
}

function generateOptionsYaml(feeds, exponents) {
  const lines = [];

  for (const feed of feeds) {
    const base = feed.attributes.base.toUpperCase();
    const quote = feed.attributes.quote_currency.toUpperCase();
    const id = generateId(base, quote);
    const label = generateLabel(base, quote);
    const value = formatFeedId(feed.id);
    const expo = exponents[feed.id];
    const decimals = expo !== undefined ? Math.abs(expo) : 8;

    lines.push(`      - id: "${id}"`);
    lines.push(`        label: "${label}"`);
    lines.push(`        value: "${value}"`);
    lines.push(`        description: "${label} (${decimals} decimals)"`);
  }

  return lines.join('\n');
}

async function main() {
  console.log('Fetching price feeds from Hermes API...');

  const feeds = await fetchPriceFeeds();
  console.log(`Found ${feeds.length} total price feeds`);

  // Filter for crypto feeds with USD quote
  const cryptoFeeds = feeds.filter(feed => {
    const attrs = feed.attributes;
    if (attrs.asset_type !== 'Crypto') return false;

    const base = attrs.base.toUpperCase();
    const quote = attrs.quote_currency.toUpperCase();

    // Include popular tokens with USD quote
    if (quote === 'USD' && POPULAR_SYMBOLS.has(base)) return true;

    // Include ETH/BTC pair
    if (base === 'ETH' && quote === 'BTC') return true;

    return false;
  });

  console.log(`Found ${cryptoFeeds.length} popular crypto/USD feeds`);

  // Fetch exponents for all feeds
  console.log('Fetching exponents...');
  const feedIds = cryptoFeeds.map(f => f.id);
  const exponents = await fetchExponents(feedIds);
  console.log(`Got exponents for ${Object.keys(exponents).length} feeds`);

  // Sort by base symbol
  cryptoFeeds.sort((a, b) => {
    const aBase = a.attributes.base.toUpperCase();
    const bBase = b.attributes.base.toUpperCase();
    const priority = ['SOL', 'BTC', 'ETH', 'USDC', 'USDT'];
    const aIdx = priority.indexOf(aBase);
    const bIdx = priority.indexOf(bBase);
    if (aIdx !== -1 && bIdx !== -1) return aIdx - bIdx;
    if (aIdx !== -1) return -1;
    if (bIdx !== -1) return 1;
    return aBase.localeCompare(bBase);
  });

  // Read existing overrides.yaml
  console.log(`Reading ${OVERRIDES_FILE}...`);
  const yamlContent = fs.readFileSync(OVERRIDES_FILE, 'utf8');

  // Find and replace the options section
  // Match from "    options:" to just before "templates:" or end of constants block
  const optionsStartRegex = /(constants:\s+price_feed:\s+label:[^\n]*\n\s+description:[^\n]*\n)\s+options:\n([\s\S]*?)(\n\ntemplates:)/;

  const match = yamlContent.match(optionsStartRegex);
  if (!match) {
    console.error('Could not find options section in overrides.yaml');
    console.error('Expected format: constants.price_feed.options followed by templates section');
    process.exit(1);
  }

  const newOptionsYaml = generateOptionsYaml(cryptoFeeds, exponents);
  const newYamlContent = yamlContent.replace(
    optionsStartRegex,
    `$1    options:\n${newOptionsYaml}$3`
  );

  // Write updated yaml
  fs.writeFileSync(OVERRIDES_FILE, newYamlContent);
  console.log(`\nUpdated ${OVERRIDES_FILE} with ${cryptoFeeds.length} price feeds`);
}

main().catch(err => {
  console.error('Error:', err.message);
  process.exit(1);
});
