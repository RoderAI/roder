#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const path = require('node:path');

function binaryForPlatform(platform = process.platform, arch = process.arch) {
  const target = `${platform}-${arch}`;
  if (!['linux-x64', 'linux-arm64', 'darwin-arm64'].includes(target)) {
    throw new Error(`Roder does not support ${target}. Use Linux x64/ARM64 with glibc 2.35+ or macOS Apple Silicon.`);
  }
  return path.join(__dirname, target, 'roder');
}

function main(args = process.argv.slice(2)) {
  let binary;
  try {
    binary = binaryForPlatform();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
    return;
  }
  const child = spawn(binary, args, { stdio: 'inherit' });
  const handlers = new Map();
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    const handler = () => child.kill(signal);
    handlers.set(signal, handler);
    process.on(signal, handler);
  }
  const cleanup = () => {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
  };
  child.on('error', (error) => {
    cleanup();
    console.error(`Could not start Roder: ${error.message}. Reinstall @roderai/roder.`);
    process.exitCode = 1;
  });
  child.on('exit', (code, signal) => {
    cleanup();
    if (signal) process.kill(process.pid, signal);
    else process.exitCode = code;
  });
}

module.exports = { binaryForPlatform, main };
if (require.main === module) main();
