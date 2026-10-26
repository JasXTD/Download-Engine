#!/usr/bin/env bash
set -euo pipefail

# Run in the extracted, pinned FFmpeg source tree. Use the existing sed tool so
# rebuilding this small downstream patch needs no additional patching package.
# Both input and output hashes make an upstream drift or partial edit fatal.
sha256sum --check <<'BEFORE'
da459d3113d590cffbcea6f14ad55de20a32869cf8979eea17ab3bf83e2641ba  libavutil/encryption_info.c
533674687505d25db7176cf876036f600e6d8cb25079ffa6b28b4a8e8266eae1  libavformat/mov.c
a9472acb3fc8cda87b5f5b0cea91cb681dbf6a6459f7569682f6e3c3a6605be0  libavutil/hdr_dynamic_metadata.c
BEFORE

# CVE-2026-107678: free the initialization list without recursive stack growth.
sed -i '/^void av_encryption_init_info_free(/,/^}/c\
void av_encryption_init_info_free(AVEncryptionInitInfo *info)\
{\
    uint32_t i;\
    while (info) {\
        AVEncryptionInitInfo *next = info->next;\
        for (i = 0; i < info->num_key_ids; i++) {\
            av_free(info->key_ids[i]);\
        }\
        av_free(info->system_id);\
        av_free(info->key_ids);\
        av_free(info->data);\
        av_free(info);\
        info = next;\
    }\
}' libavutil/encryption_info.c

# This helper supports clear media only. Refuse PSSH at the demuxer before
# allocation/list copying, including progressive MP4 and fragment metadata.
sed -i '/^static int mov_read_pssh(/,/^}/c\
static int mov_read_pssh(MOVContext *c, AVIOContext *pb, MOVAtom atom)\
{\
    av_log(c->fc, AV_LOG_ERROR, "XDownloader does not support encrypted / DRM media (pssh).\\n");\
    return AVERROR_INVALIDDATA;\
}' libavformat/mov.c

# CVE-2026-107676, upstream 2c2f6e96e31795ae95a8f8323a493ffbc493111f:
# saturation fields exist even without tone mapping. Write all sized fields.
sed -i '/^            put_bits(pb, 1, s->params\[w\].color_saturation_mapping_flag);$/,/^        }$/c\
        }\
        put_bits(pb, 1, s->params[w].color_saturation_mapping_flag);\
        if (s->params[w].color_saturation_mapping_flag)\
            put_bits(pb, 6, s->params[w].color_saturation_weight.num * saturation_weight_den /\
                s->params[w].color_saturation_weight.den);' libavutil/hdr_dynamic_metadata.c

sha256sum --check <<'AFTER'
943523ec3e4090a213ae1d3aa78c733ab1bda4229dfcf18a666cd0576c0af74c  libavutil/encryption_info.c
1a39bfab027e5eace3b7af38c56c9da6cc43a341ae5719f27e6bda1458cdfb8f  libavformat/mov.c
23df4161f23452d5a5c1c586fee6e0b193e0df701ffb611d95c50a8b4fb0fcfe  libavutil/hdr_dynamic_metadata.c
AFTER
