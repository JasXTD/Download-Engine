/* Check initialization on the same parser object linked into the media helper. */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include "libavcodec/parser_internal.h"
#include "libavutil/log.h"
#include "libavutil/mem.h"

extern const FFCodecParser ff_vorbis_parser;
static unsigned errors;

static void count_errors(void *ctx, int level, const char *fmt, va_list args)
{
    (void)ctx;
    (void)fmt;
    (void)args;
    if (level <= AV_LOG_ERROR)
        errors++;
}

int main(void)
{
    uint8_t empty_header[1 + AV_INPUT_BUFFER_PADDING_SIZE] = {0};
    uint8_t packet[1 + AV_INPUT_BUFFER_PADDING_SIZE] = {0};
    AVCodecContext codec = {.extradata = empty_header, .extradata_size = 1};
    AVCodecParserContext parser = {0};
    parser.priv_data = av_mallocz(ff_vorbis_parser.priv_data_size);
    if (!parser.priv_data)
        return 1;
    av_log_set_callback(count_errors);
    unsigned first = 0;
    int passed = 1;
    for (int i = 0; i < 2; i++) {
        const uint8_t *out = NULL;
        int size = 0;
        int used = ff_vorbis_parser.parse(&parser, &codec, &out, &size, packet, 1);
        passed &= used == 1 && size == 1 && out == packet;
        if (i == 0)
            first = errors;
    }
    av_free(parser.priv_data);
    printf("Vorbis header diagnostics after first and second packet: %u, %u\n", first, errors);
    return passed && first > 0 && errors == first ? 0 : 1;
}
