use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::metadata::{
    AccountKindData, AlbumArtistData, AlbumData, AppleIdData, ArtistData, ArtistIdData, CmIdData,
    CnIdData, CommentData, CompilationData, ComposerData, CopyrightData, DATA_TYPE_BINARY,
    DATA_TYPE_FLOAT32_BIG_ENDIAN, DATA_TYPE_FLOAT64_BIG_ENDIAN, DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
    DATA_TYPE_STRING_JPEG, DATA_TYPE_STRING_MAC, DATA_TYPE_STRING_UTF8, DATA_TYPE_STRING_UTF16,
    Data, DateData, DescriptionData, DiskNumberData, EncodingToolData, EpisodeGuidData,
    GaplessPlaybackData, GenreData, GenreIdData, GroupingData, Ilst, IlstMetaContainer, Key, Keys,
    LegacyGenreData, MediaTypeData, NameData, NumberedMetadataItem, PlaylistIdData, PodcastData,
    PodcastUrlData, PurchaseDateData, RatingData, SfIdData, SortAlbumArtistData, SortAlbumData,
    SortArtistData, SortComposerData, SortNameData, SortShowData, StringData, TempoData,
    TrackNumberData, TvEpisodeData, TvEpisodeIdData, TvNetworkNameData, TvSeasonData,
    TvShowNameData, WriterData,
};
use mp4forge::boxes::{AnyTypeBox, default_registry};
use mp4forge::codec::{CodecBox, ImmutableBox, MutableBox, marshal, unmarshal, unmarshal_any};
use mp4forge::stringify::stringify;

fn assert_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

fn assert_box_roundtrip_with_registry<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + Default + Clone + PartialEq + Debug + 'static,
{
    assert_box_roundtrip(src.clone(), payload, expected);

    let registry = default_registry();
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        src.box_type(),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(
        any_read,
        payload.len() as u64,
        "registry unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);
}

fn assert_any_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + AnyTypeBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    decoded.set_box_type(src.box_type());
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

#[test]
fn metadata_catalog_roundtrips() {
    assert_box_roundtrip_with_registry(Ilst, &[], "");

    assert_any_box_roundtrip(IlstMetaContainer::default(), &[], "");

    let data_cases = [
        (
            Data {
                data_type: DATA_TYPE_BINARY,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=BINARY DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_STRING_UTF8,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=UTF8 DataLang=305419896 Data=\"foo\"",
        ),
        (
            Data {
                data_type: DATA_TYPE_STRING_UTF8,
                data_lang: 0x12345678,
                data: vec![0x00, b'f', b'o', b'o'],
            },
            vec![
                0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x00, 0x66, 0x6f, 0x6f,
            ],
            "DataType=UTF8 DataLang=305419896 Data=\".foo\"",
        ),
        (
            Data {
                data_type: DATA_TYPE_STRING_UTF16,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x02, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=UTF16 DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_STRING_MAC,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x03, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=MAC_STR DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_STRING_JPEG,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x0e, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=JPEG DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=INT DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_FLOAT32_BIG_ENDIAN,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x16, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=FLOAT32 DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
        (
            Data {
                data_type: DATA_TYPE_FLOAT64_BIG_ENDIAN,
                data_lang: 0x12345678,
                data: b"foo".to_vec(),
            },
            vec![
                0x00, 0x00, 0x00, 0x17, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
            ],
            "DataType=FLOAT64 DataLang=305419896 Data=[0x66, 0x6f, 0x6f]",
        ),
    ];

    for (src, payload, expected) in data_cases {
        assert_box_roundtrip(src, &payload, expected);
    }

    let mut mean = StringData::default();
    mean.set_box_type(FourCc::from_bytes(*b"mean"));
    mean.data = vec![0x00, b'f', b'o', b'o'];
    assert_any_box_roundtrip(mean, &[0x00, 0x66, 0x6f, 0x6f], "Data=\".foo\"");

    let mut name = StringData::default();
    name.set_box_type(FourCc::from_bytes(*b"name"));
    name.data = b"Album".to_vec();
    assert_any_box_roundtrip(name, b"Album", "Data=\"Album\"");

    let mut numbered = NumberedMetadataItem::default();
    numbered.set_box_type(FourCc::from_u32(1));
    numbered.set_version(0);
    numbered.set_flags(0);
    numbered.item_name = FourCc::from_bytes(*b"data");
    numbered.data = Data {
        data_type: DATA_TYPE_BINARY,
        data_lang: 0x12345678,
        data: b"foo".to_vec(),
    };

    let numbered_payload = [
        0x00, 0x00, 0x00, 0x00, b'd', b'a', b't', b'a', 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56,
        0x78, 0x66, 0x6f, 0x6f,
    ];
    let mut decoded_numbered = NumberedMetadataItem::default();
    decoded_numbered.set_box_type(FourCc::from_u32(1));
    let mut reader = Cursor::new(numbered_payload);
    let read = unmarshal(
        &mut reader,
        numbered_payload.len() as u64,
        &mut decoded_numbered,
        None,
    )
    .unwrap();
    assert_eq!(read, numbered_payload.len() as u64);
    assert_eq!(decoded_numbered, numbered);
    assert_eq!(
        stringify(&numbered, None).unwrap(),
        "Version=0 Flags=0x000000 ItemName=\"data\" Data={DataType=BINARY DataLang=305419896 Data=[0x66, 0x6f, 0x6f]}"
    );

    let mut encoded_numbered = Vec::new();
    let numbered_written = marshal(&mut encoded_numbered, &numbered, None).unwrap();
    assert_eq!(numbered_written, numbered_payload.len() as u64);
    assert_eq!(encoded_numbered, numbered_payload);

    let nested_numbered_payload = [
        0x00, 0x00, 0x00, 0x15, b'd', b'a', b't', b'a', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x31, 0x2e, 0x30, 0x2e, 0x30,
    ];
    let mut decoded_nested_numbered = NumberedMetadataItem::default();
    decoded_nested_numbered.set_box_type(FourCc::from_u32(1));
    let mut reader = Cursor::new(nested_numbered_payload);
    let read = unmarshal(
        &mut reader,
        nested_numbered_payload.len() as u64,
        &mut decoded_nested_numbered,
        None,
    )
    .unwrap();
    assert_eq!(read, nested_numbered_payload.len() as u64);
    assert_eq!(decoded_nested_numbered.version(), 0);
    assert_eq!(decoded_nested_numbered.flags(), 0);
    assert_eq!(
        decoded_nested_numbered.item_name,
        FourCc::from_bytes(*b"data")
    );
    assert_eq!(
        decoded_nested_numbered.data.data_type,
        DATA_TYPE_STRING_UTF8
    );
    assert_eq!(decoded_nested_numbered.data.data_lang, 0);
    assert_eq!(decoded_nested_numbered.data.data, b"1.0.0");
    assert_eq!(
        stringify(&decoded_nested_numbered, None).unwrap(),
        "Version=0 Flags=0x000000 ItemName=\"data\" Data={DataType=UTF8 DataLang=0 Data=\"1.0.0\"}"
    );

    let mut reencoded_nested_numbered = Vec::new();
    let nested_written = marshal(
        &mut reencoded_nested_numbered,
        &decoded_nested_numbered,
        None,
    )
    .unwrap();
    assert_eq!(nested_written, nested_numbered_payload.len() as u64);
    assert_eq!(reencoded_nested_numbered, nested_numbered_payload);

    let mut keys = Keys::default();
    keys.set_version(0);
    keys.entry_count = 2;
    keys.entries = vec![
        Key {
            key_size: 27,
            key_namespace: FourCc::from_bytes(*b"mdta"),
            key_value: b"com.android.version".to_vec(),
        },
        Key {
            key_size: 25,
            key_namespace: FourCc::from_bytes(*b"mdta"),
            key_value: b"com.android.model".to_vec(),
        },
    ];

    let keys_payload = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x1b, 0x6d, 0x64, 0x74,
        0x61, 0x63, 0x6f, 0x6d, 0x2e, 0x61, 0x6e, 0x64, 0x72, 0x6f, 0x69, 0x64, 0x2e, 0x76, 0x65,
        0x72, 0x73, 0x69, 0x6f, 0x6e, 0x00, 0x00, 0x00, 0x19, 0x6d, 0x64, 0x74, 0x61, 0x63, 0x6f,
        0x6d, 0x2e, 0x61, 0x6e, 0x64, 0x72, 0x6f, 0x69, 0x64, 0x2e, 0x6d, 0x6f, 0x64, 0x65, 0x6c,
    ];
    assert_box_roundtrip_with_registry(
        keys,
        &keys_payload,
        "Version=0 Flags=0x000000 EntryCount=2 Entries=[{KeySize=27 KeyNamespace=\"mdta\" KeyValue=\"com.android.version\"}, {KeySize=25 KeyNamespace=\"mdta\" KeyValue=\"com.android.model\"}]",
    );
}

#[test]
fn tuple_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        TrackNumberData {
            data_type: DATA_TYPE_BINARY,
            data_lang: 0x12345678,
            leading_reserved: 0,
            track_number: 7,
            total_tracks: 9,
            trailing_reserved: 0,
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x07, 0x00, 0x09,
            0x00, 0x00,
        ],
        "DataType=BINARY DataLang=305419896 TrackNumber=7 TotalTracks=9 LeadingReserved=0 TrailingReserved=0",
    );

    assert_box_roundtrip(
        DiskNumberData {
            data_type: DATA_TYPE_BINARY,
            data_lang: 0x12345678,
            leading_reserved: 0,
            disk_number: 2,
            total_disks: 3,
            trailing_reserved: 0,
        },
        &[
            0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x02, 0x00, 0x03,
            0x00, 0x00,
        ],
        "DataType=BINARY DataLang=305419896 DiskNumber=2 TotalDisks=3 LeadingReserved=0 TrailingReserved=0",
    );
}

#[test]
fn scalar_and_boolean_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        TempoData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tempo: 120,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x78],
        "DataType=INT DataLang=305419896 Tempo=120",
    );

    assert_box_roundtrip(
        MediaTypeData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            media_type: 10,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x0a],
        "DataType=INT DataLang=305419896 MediaType=10",
    );

    assert_box_roundtrip(
        CompilationData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_compilation: true,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01],
        "DataType=INT DataLang=305419896 Compilation=true",
    );

    assert_box_roundtrip(
        PodcastData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_podcast: true,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01],
        "DataType=INT DataLang=305419896 Podcast=true",
    );

    assert_box_roundtrip(
        GaplessPlaybackData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_gapless_playback: true,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01],
        "DataType=INT DataLang=305419896 GaplessPlayback=true",
    );

    assert_box_roundtrip(
        RatingData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            rating: 4,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x04],
        "DataType=INT DataLang=305419896 Rating=4",
    );
}

#[test]
fn store_identifier_and_purchase_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        AccountKindData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            account_kind: 2,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x02],
        "DataType=INT DataLang=305419896 AccountKind=2",
    );

    assert_box_roundtrip(
        AppleIdData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            apple_id: b"123456789".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
            0x37, 0x38, 0x39,
        ],
        "DataType=UTF8 DataLang=305419896 AppleId=\"123456789\"",
    );

    assert_box_roundtrip(
        ArtistIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            artist_id: 42,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x2a,
        ],
        "DataType=INT DataLang=305419896 ArtistId=42",
    );

    assert_box_roundtrip(
        CmIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            cmid: 314159,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x04, 0xcb, 0x2f,
        ],
        "DataType=INT DataLang=305419896 CmId=314159",
    );

    assert_box_roundtrip(
        CnIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            cnid: 65537,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x01, 0x00, 0x01,
        ],
        "DataType=INT DataLang=305419896 CnId=65537",
    );

    assert_box_roundtrip(
        EpisodeGuidData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            episode_guid: b"episode-guid-1".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x65, 0x70, 0x69, 0x73, 0x6f, 0x64,
            0x65, 0x2d, 0x67, 0x75, 0x69, 0x64, 0x2d, 0x31,
        ],
        "DataType=UTF8 DataLang=305419896 EpisodeGuid=\"episode-guid-1\"",
    );

    assert_box_roundtrip(
        GenreIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            genre_id: 17,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x11,
        ],
        "DataType=INT DataLang=305419896 GenreId=17",
    );

    assert_box_roundtrip(
        PlaylistIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            playlist_id: 4_294_967_298,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x02,
        ],
        "DataType=INT DataLang=305419896 PlaylistId=4294967298",
    );

    assert_box_roundtrip(
        PurchaseDateData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            purchase_date: b"2026-04-20T10:30:00Z".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x32, 0x30, 0x32, 0x36, 0x2d, 0x30,
            0x34, 0x2d, 0x32, 0x30, 0x54, 0x31, 0x30, 0x3a, 0x33, 0x30, 0x3a, 0x30, 0x30, 0x5a,
        ],
        "DataType=UTF8 DataLang=305419896 PurchaseDate=\"2026-04-20T10:30:00Z\"",
    );

    assert_box_roundtrip(
        PodcastUrlData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            podcast_url: b"https://example.invalid/feed".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x68, 0x74, 0x74, 0x70, 0x73, 0x3a,
            0x2f, 0x2f, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x61,
            0x6c, 0x69, 0x64, 0x2f, 0x66, 0x65, 0x65, 0x64,
        ],
        "DataType=UTF8 DataLang=305419896 PodcastUrl=\"https://example.invalid/feed\"",
    );

    assert_box_roundtrip(
        SfIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            sfid: 143,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x8f,
        ],
        "DataType=INT DataLang=305419896 SfId=143",
    );
}

#[test]
fn descriptive_and_credit_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        AlbumArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            album_artist: b"The Beatles".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x54, 0x68, 0x65, 0x20, 0x42, 0x65,
            0x61, 0x74, 0x6c, 0x65, 0x73,
        ],
        "DataType=UTF8 DataLang=305419896 AlbumArtist=\"The Beatles\"",
    );

    assert_box_roundtrip(
        DescriptionData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            description: b"Remastered".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x52, 0x65, 0x6d, 0x61, 0x73, 0x74,
            0x65, 0x72, 0x65, 0x64,
        ],
        "DataType=UTF8 DataLang=305419896 Description=\"Remastered\"",
    );

    assert_box_roundtrip(
        AlbumData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            album: b"Abbey Road".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x41, 0x62, 0x62, 0x65, 0x79, 0x20,
            0x52, 0x6f, 0x61, 0x64,
        ],
        "DataType=UTF8 DataLang=305419896 Album=\"Abbey Road\"",
    );

    assert_box_roundtrip(
        ArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            artist: b"The Beatles".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x54, 0x68, 0x65, 0x20, 0x42, 0x65,
            0x61, 0x74, 0x6c, 0x65, 0x73,
        ],
        "DataType=UTF8 DataLang=305419896 Artist=\"The Beatles\"",
    );

    assert_box_roundtrip(
        CommentData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            comment: b"Mono mix".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4d, 0x6f, 0x6e, 0x6f, 0x20, 0x6d,
            0x69, 0x78,
        ],
        "DataType=UTF8 DataLang=305419896 Comment=\"Mono mix\"",
    );

    assert_box_roundtrip(
        ComposerData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            composer: b"George Harrison".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x47, 0x65, 0x6f, 0x72, 0x67, 0x65,
            0x20, 0x48, 0x61, 0x72, 0x72, 0x69, 0x73, 0x6f, 0x6e,
        ],
        "DataType=UTF8 DataLang=305419896 Composer=\"George Harrison\"",
    );

    assert_box_roundtrip(
        CopyrightData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            copyright: b"(c) Example Studio".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x28, 0x63, 0x29, 0x20, 0x45, 0x78,
            0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x53, 0x74, 0x75, 0x64, 0x69, 0x6f,
        ],
        "DataType=UTF8 DataLang=305419896 Copyright=\"(c) Example Studio\"",
    );

    assert_box_roundtrip(
        DateData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            date: b"1969-09-26".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31, 0x39, 0x36, 0x39, 0x2d, 0x30,
            0x39, 0x2d, 0x32, 0x36,
        ],
        "DataType=UTF8 DataLang=305419896 Date=\"1969-09-26\"",
    );

    assert_box_roundtrip(
        GenreData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            genre: b"Rock".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x52, 0x6f, 0x63, 0x6b,
        ],
        "DataType=UTF8 DataLang=305419896 Genre=\"Rock\"",
    );

    assert_box_roundtrip(
        LegacyGenreData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            legacy_genre: 13,
        },
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x0d],
        "DataType=INT DataLang=305419896 LegacyGenre=13",
    );

    assert_box_roundtrip(
        GroupingData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            grouping: b"Side A".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x53, 0x69, 0x64, 0x65, 0x20, 0x41,
        ],
        "DataType=UTF8 DataLang=305419896 Grouping=\"Side A\"",
    );

    assert_box_roundtrip(
        NameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            name: b"Come Together".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x43, 0x6f, 0x6d, 0x65, 0x20, 0x54,
            0x6f, 0x67, 0x65, 0x74, 0x68, 0x65, 0x72,
        ],
        "DataType=UTF8 DataLang=305419896 Name=\"Come Together\"",
    );

    assert_box_roundtrip(
        EncodingToolData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            encoding_tool: b"Example Encoder 1.0".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x45, 0x78, 0x61, 0x6d, 0x70, 0x6c,
            0x65, 0x20, 0x45, 0x6e, 0x63, 0x6f, 0x64, 0x65, 0x72, 0x20, 0x31, 0x2e, 0x30,
        ],
        "DataType=UTF8 DataLang=305419896 EncodingTool=\"Example Encoder 1.0\"",
    );

    assert_box_roundtrip(
        WriterData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            writer: b"Lennon-McCartney".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4c, 0x65, 0x6e, 0x6e, 0x6f, 0x6e,
            0x2d, 0x4d, 0x63, 0x43, 0x61, 0x72, 0x74, 0x6e, 0x65, 0x79,
        ],
        "DataType=UTF8 DataLang=305419896 Writer=\"Lennon-McCartney\"",
    );
}

#[test]
fn sort_order_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        SortAlbumArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_album_artist: b"Beatles".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x42, 0x65, 0x61, 0x74, 0x6c, 0x65,
            0x73,
        ],
        "DataType=UTF8 DataLang=305419896 SortAlbumArtist=\"Beatles\"",
    );

    assert_box_roundtrip(
        SortAlbumData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_album: b"Abbey Road".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x41, 0x62, 0x62, 0x65, 0x79, 0x20,
            0x52, 0x6f, 0x61, 0x64,
        ],
        "DataType=UTF8 DataLang=305419896 SortAlbum=\"Abbey Road\"",
    );

    assert_box_roundtrip(
        SortArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_artist: b"Lennon, John".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4c, 0x65, 0x6e, 0x6e, 0x6f, 0x6e,
            0x2c, 0x20, 0x4a, 0x6f, 0x68, 0x6e,
        ],
        "DataType=UTF8 DataLang=305419896 SortArtist=\"Lennon, John\"",
    );

    assert_box_roundtrip(
        SortComposerData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_composer: b"McCartney, Paul".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4d, 0x63, 0x43, 0x61, 0x72, 0x74,
            0x6e, 0x65, 0x79, 0x2c, 0x20, 0x50, 0x61, 0x75, 0x6c,
        ],
        "DataType=UTF8 DataLang=305419896 SortComposer=\"McCartney, Paul\"",
    );

    assert_box_roundtrip(
        SortNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_name: b"Come Together".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x43, 0x6f, 0x6d, 0x65, 0x20, 0x54,
            0x6f, 0x67, 0x65, 0x74, 0x68, 0x65, 0x72,
        ],
        "DataType=UTF8 DataLang=305419896 SortName=\"Come Together\"",
    );

    assert_box_roundtrip(
        SortShowData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_show: b"Beatles Anthology".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x42, 0x65, 0x61, 0x74, 0x6c, 0x65,
            0x73, 0x20, 0x41, 0x6e, 0x74, 0x68, 0x6f, 0x6c, 0x6f, 0x67, 0x79,
        ],
        "DataType=UTF8 DataLang=305419896 SortShow=\"Beatles Anthology\"",
    );
}

#[test]
fn television_and_show_identification_metadata_value_boxes_roundtrip() {
    assert_box_roundtrip(
        TvEpisodeIdData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_episode_id: b"1".to_vec(),
        },
        &[0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31],
        "DataType=UTF8 DataLang=305419896 TvEpisodeId=\"1\"",
    );

    assert_box_roundtrip(
        TvEpisodeData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tv_episode: 12,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x0c,
        ],
        "DataType=INT DataLang=305419896 TvEpisode=12",
    );

    assert_box_roundtrip(
        TvNetworkNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_network_name: b"HBO".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x48, 0x42, 0x4f,
        ],
        "DataType=UTF8 DataLang=305419896 TvNetworkName=\"HBO\"",
    );

    assert_box_roundtrip(
        TvShowNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_show_name: b"Example Show".to_vec(),
        },
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x45, 0x78, 0x61, 0x6d, 0x70, 0x6c,
            0x65, 0x20, 0x53, 0x68, 0x6f, 0x77,
        ],
        "DataType=UTF8 DataLang=305419896 TvShowName=\"Example Show\"",
    );

    assert_box_roundtrip(
        TvSeasonData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tv_season: 5,
        },
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x05,
        ],
        "DataType=INT DataLang=305419896 TvSeason=5",
    );
}

#[test]
fn boolean_metadata_value_boxes_reject_out_of_range_flags() {
    let payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x02];

    let mut decoded = CompilationData::default();
    let mut reader = Cursor::new(payload);
    let error = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Compilation: value must be 0 or 1"
    );

    let mut decoded = GaplessPlaybackData::default();
    let mut reader = Cursor::new(payload);
    let error = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for GaplessPlayback: value must be 0 or 1"
    );
}

#[test]
fn keys_reject_truncated_entry_payloads() {
    let payload = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x1b, 0x6d, 0x64, 0x74,
        0x61, 0x63, 0x6f, 0x6d, 0x2e, 0x61, 0x6e, 0x64, 0x72,
    ];

    let mut decoded = Keys::default();
    let mut reader = Cursor::new(payload);
    let error = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Entries: entry payload length does not match the entry count"
    );
}

#[test]
fn built_in_registry_reports_context_free_metadata_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"ilst")),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"keys")),
        Some(&[][..])
    );

    assert!(registry.is_registered(FourCc::from_bytes(*b"ilst")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"keys")));
    assert!(!registry.is_registered(FourCc::from_bytes(*b"data")));
    assert!(!registry.is_registered(FourCc::from_bytes(*b"----")));
    assert!(!registry.is_registered(FourCc::from_bytes(*b"mean")));
    assert!(!registry.is_registered(FourCc::from_bytes(*b"name")));
    assert!(!registry.is_registered(FourCc::from_u32(1)));
}
