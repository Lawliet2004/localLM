# LocalLM

<!-- impeccable:product-schema 1 -->

## Platform

web

React and TypeScript UI inside a Tauri 2 desktop application. Browser preview exposes the interface; native operations require the desktop app.

## Product Purpose

A workspace for conversations, local model inference, explicitly selected API models, project files, and permission-controlled tools. Source: README.md and the current implementation.

## Operating Context

Conversations belong to projects. Users configure local GGUF models and llama.cpp, or connect a provider. Review, terminal, browser, and editor panels support work inside the app. General settings, models, tools, execution, connectors, and skills are contextual settings destinations.

## Capabilities and Constraints

- Preserve model configuration, provider selection, tool approval, projects, attachments, conversation history, and drafts.
- Theme is stored on this device. Permissions follow the current conversation and the existing save path.
- Browser preview must accurately explain which operations require the desktop application.
- Local inference and explicitly selected external providers have different data paths. Do not claim all processing is local.
- Fonts ship with the application and do not require external font requests.

## Brand Commitments

LocalLM / localLM is the existing product name. Keep the L mark and model dot recognizable. The user requested a complete clean redesign of home and settings using Impeccable; no additional brand constraints were specified.

## Explicit exclusions

The homepage and chat composer have no response-presentation selector (`Default (auto)`, Auto, Text, Visual). The user removed it on 21 October 2026; configure response presentation in Settings only. Do not restore this control from historical plans, audits or logs.

## Open Decisions

Audience segmentation and any product-specific accessibility requirements have not been confirmed. The implementation supports keyboard focus, reduced motion, light and dark appearance, and narrow windows.
