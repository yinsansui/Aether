use serde_json::Value;

use crate::ai_serving::{
    maybe_build_ai_surface_stream_rewriter, AiSurfaceFinalizeError, AiSurfaceStreamRewriter,
    ResponseHistoryRecord,
};
use crate::GatewayError;

pub(crate) struct LocalStreamRewriter<'a> {
    inner: Option<AiSurfaceStreamRewriter<'a>>,
    report_context: &'a Value,
    /// Client-visible SSE held until finish so compact synthesis can atomically
    /// replace the stream with a single compaction item.
    synthesis_buffer: Vec<u8>,
    needs_compact_synthesis: bool,
    /// True once any non-empty bytes were released downstream. When set, empty
    /// synthesis must not escalate to Internal/502.
    released_downstream: bool,
}

pub(crate) fn maybe_build_local_stream_rewriter<'a>(
    report_context: Option<&'a Value>,
) -> Option<LocalStreamRewriter<'a>> {
    let report_context = report_context?;
    let inner = maybe_build_ai_surface_stream_rewriter(Some(report_context));
    let needs_compact_synthesis =
        aether_ai_formats::compact_synthesis_enabled_from_report_context(Some(report_context))
            && aether_ai_formats::is_compact_operation_from_report_context(Some(report_context));
    if inner.is_none() && !needs_compact_synthesis {
        return None;
    }
    Some(LocalStreamRewriter {
        inner,
        report_context,
        synthesis_buffer: Vec::new(),
        needs_compact_synthesis,
        released_downstream: false,
    })
}

impl LocalStreamRewriter<'_> {
    pub(crate) fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<u8>, GatewayError> {
        let transformed = if let Some(inner) = self.inner.as_mut() {
            inner.push_chunk(chunk).map_err(map_surface_error)?
        } else {
            chunk.to_vec()
        };
        if self.needs_compact_synthesis {
            // Hold all client-visible bytes until finish so synthesis can
            // atomically emit a single compaction SSE. Prefetch drop/rebuild
            // replays provider bytes into a fresh rewriter; buffering here is
            // restored by that replay.
            self.synthesis_buffer.extend_from_slice(&transformed);
            return Ok(Vec::new());
        }
        if !transformed.is_empty() {
            self.released_downstream = true;
        }
        Ok(transformed)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<u8>, GatewayError> {
        let trailing = if let Some(inner) = self.inner.as_mut() {
            inner.finish().map_err(map_surface_error)?
        } else {
            Vec::new()
        };
        if self.needs_compact_synthesis {
            self.synthesis_buffer.extend_from_slice(&trailing);
            let bytes = std::mem::take(&mut self.synthesis_buffer);
            return self.finish_compact_synthesis(bytes);
        }
        if !trailing.is_empty() {
            self.released_downstream = true;
        }
        Ok(trailing)
    }

    fn finish_compact_synthesis(&mut self, bytes: Vec<u8>) -> Result<Vec<u8>, GatewayError> {
        let Ok(sse) = std::str::from_utf8(&bytes) else {
            if !bytes.is_empty() {
                self.released_downstream = true;
            }
            return Ok(bytes);
        };
        let model = self
            .report_context
            .get("mapped_model")
            .or_else(|| self.report_context.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        match aether_ai_formats::maybe_rewrite_responses_sse_for_compact_synthesis(
            sse, model, true, true,
        ) {
            Ok(Some(rewritten)) => {
                self.released_downstream = true;
                Ok(rewritten.into_bytes())
            }
            Ok(None) => {
                if !bytes.is_empty() {
                    self.released_downstream = true;
                }
                Ok(bytes)
            }
            Err(err) => {
                if self.released_downstream {
                    Ok(bytes)
                } else {
                    Err(GatewayError::Internal(err))
                }
            }
        }
    }

    pub(crate) fn take_response_history_record(&mut self) -> Option<ResponseHistoryRecord> {
        self.inner
            .as_mut()
            .and_then(|inner| inner.take_response_history_record())
    }
}

fn map_surface_error(error: AiSurfaceFinalizeError) -> GatewayError {
    error.into()
}

#[cfg(test)]
#[path = "../tests_stream.rs"]
mod tests;
