#!/bin/bash

# Build script for Boss Fight Betting Solana Program

set -e

echo "🔨 Building Boss Fight Betting Solana Program"
echo "=============================================="

# Check if Anchor is installed
if ! command -v anchor &> /dev/null; then
    echo "❌ Anchor CLI not found. Please install Anchor first:"
    echo "   npm i -g @coral-xyz/anchor-cli"
    exit 1
fi

# Check if Solana CLI is installed
if ! command -v solana &> /dev/null; then
    echo "❌ Solana CLI not found. Please install Solana CLI first:"
    echo "   sh -c \"\$(curl -sSfL https://release.solana.com/v1.16.0/install)\""
    exit 1
fi

echo "✅ Prerequisites check passed"

# Set Solana config for development
echo "🌐 Setting up Solana config for devnet..."
solana config set --url devnet
solana config set --keypair ~/.config/solana/id.json

echo "📊 Current Solana configuration:"
solana config get

# Build the program
echo "🏗️  Building Anchor program..."
anchor build

# Generate IDL
echo "📄 Generating IDL..."
mkdir -p idl
cp target/idl/boss_fight_betting.json idl/

echo "🎯 Getting program ID..."
PROGRAM_ID=$(solana address -k target/deploy/boss_fight_betting-keypair.json)
echo "Program ID: $PROGRAM_ID"

# Update Anchor.toml with the correct program ID
echo "📝 Updating Anchor.toml..."
sed -i.bak "s/BossFightBetting111111111111111111111111111/$PROGRAM_ID/g" Anchor.toml

echo "🚀 Build completed successfully!"
echo ""
echo "Next steps:"
echo "1. Fund your wallet: solana airdrop 5"
echo "2. Deploy the program: anchor deploy"
echo "3. Update your .env file with the program ID: $PROGRAM_ID"
echo "4. Run the server: npm start"