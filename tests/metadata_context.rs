use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::metadata::{
    AccountKindData, AlbumArtistData, AlbumData, AppleIdData, ArtistData, ArtistIdData, CmIdData,
    CnIdData, CommentData, CompilationData, ComposerData, CopyrightData, DATA_TYPE_BINARY,
    DATA_TYPE_SIGNED_INT_BIG_ENDIAN, DATA_TYPE_STRING_JPEG, DATA_TYPE_STRING_UTF8, Data, DateData,
    DescriptionData, DiskNumberData, EncodingToolData, EpisodeGuidData, GaplessPlaybackData,
    GenreData, GenreIdData, GroupingData, IlstMetaContainer, Keys, LegacyGenreData, MediaTypeData,
    NameData, NumberedMetadataItem, PlaylistIdData, PodcastData, PodcastUrlData, PurchaseDateData,
    RatingData, SfIdData, SortAlbumArtistData, SortAlbumData, SortArtistData, SortComposerData,
    SortNameData, SortShowData, StringData, TempoData, TrackNumberData, TvEpisodeData,
    TvEpisodeIdData, TvNetworkNameData, TvSeasonData, TvShowNameData, WriterData,
};
use mp4forge::boxes::threegpp::Udta3gppString;
use mp4forge::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry, default_registry};
use mp4forge::codec::{CodecError, unmarshal, unmarshal_any_with_context};

const ILST: FourCc = FourCc::from_bytes(*b"ilst");
const FREE_FORM: FourCc = FourCc::from_bytes(*b"----");
const COVR: FourCc = FourCc::from_bytes(*b"covr");
const AART: FourCc = FourCc::from_bytes(*b"aART");
const DESC: FourCc = FourCc::from_bytes(*b"desc");
const TRKN: FourCc = FourCc::from_bytes(*b"trkn");
const DISK: FourCc = FourCc::from_bytes(*b"disk");
const TMPO: FourCc = FourCc::from_bytes(*b"tmpo");
const STIK: FourCc = FourCc::from_bytes(*b"stik");
const CPIL: FourCc = FourCc::from_bytes(*b"cpil");
const PCST: FourCc = FourCc::from_bytes(*b"pcst");
const PGAP: FourCc = FourCc::from_bytes(*b"pgap");
const RTNG: FourCc = FourCc::from_bytes(*b"rtng");
const AKID: FourCc = FourCc::from_bytes(*b"akID");
const APID: FourCc = FourCc::from_bytes(*b"apID");
const ATID: FourCc = FourCc::from_bytes(*b"atID");
const CMID: FourCc = FourCc::from_bytes(*b"cmID");
const CNID: FourCc = FourCc::from_bytes(*b"cnID");
const CALB: FourCc = FourCc::from_bytes([0xa9, b'a', b'l', b'b']);
const CART: FourCc = FourCc::from_bytes([0xa9, b'A', b'R', b'T']);
const CCMT: FourCc = FourCc::from_bytes([0xa9, b'c', b'm', b't']);
const CCOM: FourCc = FourCc::from_bytes([0xa9, b'c', b'o', b'm']);
const CDAY: FourCc = FourCc::from_bytes([0xa9, b'd', b'a', b'y']);
const CGEN: FourCc = FourCc::from_bytes([0xa9, b'g', b'e', b'n']);
const CGRP: FourCc = FourCc::from_bytes([0xa9, b'g', b'r', b'p']);
const CNAM: FourCc = FourCc::from_bytes([0xa9, b'n', b'a', b'm']);
const CTOO: FourCc = FourCc::from_bytes([0xa9, b't', b'o', b'o']);
const EGID: FourCc = FourCc::from_bytes(*b"egid");
const GEID: FourCc = FourCc::from_bytes(*b"geID");
const SOAA: FourCc = FourCc::from_bytes(*b"soaa");
const SOAL: FourCc = FourCc::from_bytes(*b"soal");
const SOAR: FourCc = FourCc::from_bytes(*b"soar");
const SOCO: FourCc = FourCc::from_bytes(*b"soco");
const SONM: FourCc = FourCc::from_bytes(*b"sonm");
const SOSN: FourCc = FourCc::from_bytes(*b"sosn");
const PLID: FourCc = FourCc::from_bytes(*b"plID");
const PURD: FourCc = FourCc::from_bytes(*b"purd");
const PURL: FourCc = FourCc::from_bytes(*b"purl");
const SFID: FourCc = FourCc::from_bytes(*b"sfID");
const TVEN: FourCc = FourCc::from_bytes(*b"tven");
const TVES: FourCc = FourCc::from_bytes(*b"tves");
const TVNN: FourCc = FourCc::from_bytes(*b"tvnn");
const TVSH: FourCc = FourCc::from_bytes(*b"tvsh");
const TVSN: FourCc = FourCc::from_bytes(*b"tvsn");
const CWRT: FourCc = FourCc::from_bytes([0xa9, b'w', b'r', b't']);
const MEAN: FourCc = FourCc::from_bytes(*b"mean");
const NAME: FourCc = FourCc::from_bytes(*b"name");
const DATA: FourCc = FourCc::from_bytes(*b"data");
const UDTA: FourCc = FourCc::from_bytes(*b"udta");
const CPRT: FourCc = FourCc::from_bytes(*b"cprt");
const GNRE: FourCc = FourCc::from_bytes(*b"gnre");

fn sample_threegpp_string(box_type: FourCc, data: &[u8]) -> Udta3gppString {
    let mut src = Udta3gppString::default();
    src.set_box_type(box_type);
    src.language = [0x05, 0x0e, 0x07];
    src.data = data.to_vec();
    src
}

fn assert_contextual_data_box<T>(
    registry: &BoxRegistry,
    context: BoxLookupContext,
    payload: &[u8],
    expected: T,
) where
    T: PartialEq + std::fmt::Debug + 'static,
{
    let (decoded, read) = unmarshal_any_with_context(
        &mut Cursor::new(payload.to_vec()),
        payload.len() as u64,
        DATA,
        registry,
        context,
        None,
    )
    .unwrap();
    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded.as_any().downcast_ref::<T>().unwrap(), &expected);
}

#[test]
fn free_form_metadata_children_require_free_form_context() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let free_form_context = ilst_context.enter(FREE_FORM);

    assert!(!registry.is_registered(MEAN));
    assert!(!registry.is_registered(NAME));
    assert!(!registry.is_registered(DATA));
    assert!(!registry.is_registered_with_context(MEAN, ilst_context));
    assert!(!registry.is_registered_with_context(NAME, ilst_context));
    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(MEAN, free_form_context));
    assert!(registry.is_registered_with_context(NAME, free_form_context));
    assert!(registry.is_registered_with_context(DATA, free_form_context));

    let mut mean_reader = Cursor::new(vec![0x00, 0x66, 0x6f, 0x6f]);
    let (mean_box, mean_read) = unmarshal_any_with_context(
        &mut mean_reader,
        4,
        MEAN,
        &registry,
        free_form_context,
        None,
    )
    .unwrap();
    assert_eq!(mean_read, 4);

    let mut expected_mean = StringData::default();
    expected_mean.set_box_type(MEAN);
    expected_mean.data = vec![0x00, 0x66, 0x6f, 0x6f];
    assert_eq!(
        mean_box.as_any().downcast_ref::<StringData>().unwrap(),
        &expected_mean
    );

    let mut name_reader = Cursor::new(b"Album".to_vec());
    let (name_box, name_read) = unmarshal_any_with_context(
        &mut name_reader,
        5,
        NAME,
        &registry,
        free_form_context,
        None,
    )
    .unwrap();
    assert_eq!(name_read, 5);

    let mut expected_name = StringData::default();
    expected_name.set_box_type(NAME);
    expected_name.data = b"Album".to_vec();
    assert_eq!(
        name_box.as_any().downcast_ref::<StringData>().unwrap(),
        &expected_name
    );

    let data_payload = [
        0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x66, 0x6f, 0x6f,
    ];
    let mut data_reader = Cursor::new(data_payload);
    let (data_box, data_read) = unmarshal_any_with_context(
        &mut data_reader,
        data_payload.len() as u64,
        DATA,
        &registry,
        free_form_context,
        None,
    )
    .unwrap();
    assert_eq!(data_read, data_payload.len() as u64);
    assert_eq!(
        data_box.as_any().downcast_ref::<Data>().unwrap(),
        &Data {
            data_type: DATA_TYPE_BINARY,
            data_lang: 0x12345678,
            data: b"foo".to_vec(),
        }
    );

    match unmarshal_any_with_context(
        &mut Cursor::new(vec![0x00]),
        1,
        MEAN,
        &registry,
        ilst_context,
        None,
    ) {
        Err(CodecError::UnknownBoxType { box_type }) => assert_eq!(box_type, MEAN),
        Ok(_) => panic!("unexpected success for mean outside free-form scope"),
        Err(other) => panic!("unexpected error for mean outside free-form scope: {other}"),
    }
}

#[test]
fn cover_art_metadata_container_keeps_generic_image_data_without_free_form_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let cover_context = ilst_context.enter(COVR);

    assert!(!registry.is_registered_with_context(COVR, BoxLookupContext::new()));
    assert!(registry.is_registered_with_context(COVR, ilst_context));
    assert!(registry.is_registered_with_context(DATA, cover_context));
    assert!(!registry.is_registered_with_context(MEAN, cover_context));
    assert!(!registry.is_registered_with_context(NAME, cover_context));

    let (cover_box, cover_read) = unmarshal_any_with_context(
        &mut Cursor::new(Vec::<u8>::new()),
        0,
        COVR,
        &registry,
        ilst_context,
        None,
    )
    .unwrap();
    assert_eq!(cover_read, 0);

    let mut expected_cover = IlstMetaContainer::default();
    expected_cover.set_box_type(COVR);
    assert_eq!(
        cover_box
            .as_any()
            .downcast_ref::<IlstMetaContainer>()
            .unwrap(),
        &expected_cover
    );

    let data_payload = [
        0x00, 0x00, 0x00, 0x0e, 0x12, 0x34, 0x56, 0x78, 0xff, 0xd8, 0xff,
    ];
    let (data_box, data_read) = unmarshal_any_with_context(
        &mut Cursor::new(data_payload),
        data_payload.len() as u64,
        DATA,
        &registry,
        cover_context,
        None,
    )
    .unwrap();
    assert_eq!(data_read, data_payload.len() as u64);
    assert_eq!(
        data_box.as_any().downcast_ref::<Data>().unwrap(),
        &Data {
            data_type: DATA_TYPE_STRING_JPEG,
            data_lang: 0x12345678,
            data: vec![0xff, 0xd8, 0xff],
        }
    );

    match unmarshal_any_with_context(
        &mut Cursor::new(vec![0x00]),
        1,
        MEAN,
        &registry,
        cover_context,
        None,
    ) {
        Err(CodecError::UnknownBoxType { box_type }) => assert_eq!(box_type, MEAN),
        Ok(_) => panic!("unexpected success for mean outside free-form scope"),
        Err(other) => panic!("unexpected error for mean outside free-form scope: {other}"),
    }
}

#[test]
fn numbered_ilst_items_follow_the_carried_keys_entry_count() {
    let registry = default_registry();
    let keys_payload = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x1b, 0x6d, 0x64, 0x74,
        0x61, 0x63, 0x6f, 0x6d, 0x2e, 0x61, 0x6e, 0x64, 0x72, 0x6f, 0x69, 0x64, 0x2e, 0x76, 0x65,
        0x72, 0x73, 0x69, 0x6f, 0x6e, 0x00, 0x00, 0x00, 0x19, 0x6d, 0x64, 0x74, 0x61, 0x63, 0x6f,
        0x6d, 0x2e, 0x61, 0x6e, 0x64, 0x72, 0x6f, 0x69, 0x64, 0x2e, 0x6d, 0x6f, 0x64, 0x65, 0x6c,
    ];
    let mut keys = Keys::default();
    let mut keys_reader = Cursor::new(keys_payload);
    let keys_read =
        unmarshal(&mut keys_reader, keys_payload.len() as u64, &mut keys, None).unwrap();
    assert_eq!(keys_read, keys_payload.len() as u64);

    let ilst_context = BoxLookupContext::new()
        .with_metadata_keys_entry_count(keys.entry_count as usize)
        .enter(ILST);
    let first_item = FourCc::from_u32(1);
    let second_item = FourCc::from_u32(2);
    let third_item = FourCc::from_u32(3);

    assert!(registry.is_registered_with_context(first_item, ilst_context));
    assert!(registry.is_registered_with_context(second_item, ilst_context));
    assert!(!registry.is_registered_with_context(third_item, ilst_context));

    let numbered_payload = [
        0x00, 0x00, 0x00, 0x00, b'd', b'a', b't', b'a', 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56,
        0x78, 0x66, 0x6f, 0x6f,
    ];
    let mut reader = Cursor::new(numbered_payload);
    let (decoded, read) = unmarshal_any_with_context(
        &mut reader,
        numbered_payload.len() as u64,
        first_item,
        &registry,
        ilst_context,
        None,
    )
    .unwrap();
    assert_eq!(read, numbered_payload.len() as u64);

    let mut expected = NumberedMetadataItem::default();
    expected.set_box_type(first_item);
    expected.item_name = DATA;
    expected.data = Data {
        data_type: DATA_TYPE_BINARY,
        data_lang: 0x12345678,
        data: b"foo".to_vec(),
    };
    assert_eq!(
        decoded
            .as_any()
            .downcast_ref::<NumberedMetadataItem>()
            .unwrap(),
        &expected
    );

    match unmarshal_any_with_context(
        &mut Cursor::new(Vec::<u8>::new()),
        0,
        third_item,
        &registry,
        ilst_context,
        None,
    ) {
        Err(CodecError::UnknownBoxType { box_type }) => assert_eq!(box_type, third_item),
        Ok(_) => panic!("unexpected success for numbered item outside keys range"),
        Err(other) => panic!("unexpected error for numbered item outside keys range: {other}"),
    }
}

#[test]
fn tuple_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let track_context = ilst_context.enter(TRKN);
    let disk_context = ilst_context.enter(DISK);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(TRKN, ilst_context));
    assert!(registry.is_registered_with_context(DISK, ilst_context));
    assert!(registry.is_registered_with_context(DATA, track_context));
    assert!(registry.is_registered_with_context(DATA, disk_context));

    let track_payload = [
        0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x07, 0x00, 0x09, 0x00,
        0x00,
    ];
    let (track_box, track_read) = unmarshal_any_with_context(
        &mut Cursor::new(track_payload),
        track_payload.len() as u64,
        DATA,
        &registry,
        track_context,
        None,
    )
    .unwrap();
    assert_eq!(track_read, track_payload.len() as u64);
    assert_eq!(
        track_box
            .as_any()
            .downcast_ref::<TrackNumberData>()
            .unwrap(),
        &TrackNumberData {
            data_type: DATA_TYPE_BINARY,
            data_lang: 0x12345678,
            leading_reserved: 0,
            track_number: 7,
            total_tracks: 9,
            trailing_reserved: 0,
        }
    );

    let disk_payload = [
        0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x02, 0x00, 0x03, 0x00,
        0x00,
    ];
    let (disk_box, disk_read) = unmarshal_any_with_context(
        &mut Cursor::new(disk_payload),
        disk_payload.len() as u64,
        DATA,
        &registry,
        disk_context,
        None,
    )
    .unwrap();
    assert_eq!(disk_read, disk_payload.len() as u64);
    assert_eq!(
        disk_box.as_any().downcast_ref::<DiskNumberData>().unwrap(),
        &DiskNumberData {
            data_type: DATA_TYPE_BINARY,
            data_lang: 0x12345678,
            leading_reserved: 0,
            disk_number: 2,
            total_disks: 3,
            trailing_reserved: 0,
        }
    );
}

#[test]
fn scalar_and_boolean_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let tempo_context = ilst_context.enter(TMPO);
    let media_type_context = ilst_context.enter(STIK);
    let compilation_context = ilst_context.enter(CPIL);
    let podcast_context = ilst_context.enter(PCST);
    let gapless_context = ilst_context.enter(PGAP);
    let rating_context = ilst_context.enter(RTNG);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(TMPO, ilst_context));
    assert!(registry.is_registered_with_context(STIK, ilst_context));
    assert!(registry.is_registered_with_context(CPIL, ilst_context));
    assert!(registry.is_registered_with_context(PCST, ilst_context));
    assert!(registry.is_registered_with_context(PGAP, ilst_context));
    assert!(registry.is_registered_with_context(RTNG, ilst_context));
    assert!(registry.is_registered_with_context(DATA, tempo_context));
    assert!(registry.is_registered_with_context(DATA, media_type_context));
    assert!(registry.is_registered_with_context(DATA, compilation_context));
    assert!(registry.is_registered_with_context(DATA, podcast_context));
    assert!(registry.is_registered_with_context(DATA, gapless_context));
    assert!(registry.is_registered_with_context(DATA, rating_context));

    let tempo_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x78];
    let (tempo_box, tempo_read) = unmarshal_any_with_context(
        &mut Cursor::new(tempo_payload),
        tempo_payload.len() as u64,
        DATA,
        &registry,
        tempo_context,
        None,
    )
    .unwrap();
    assert_eq!(tempo_read, tempo_payload.len() as u64);
    assert_eq!(
        tempo_box.as_any().downcast_ref::<TempoData>().unwrap(),
        &TempoData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tempo: 120,
        }
    );

    let media_type_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x0a];
    let (media_type_box, media_type_read) = unmarshal_any_with_context(
        &mut Cursor::new(media_type_payload),
        media_type_payload.len() as u64,
        DATA,
        &registry,
        media_type_context,
        None,
    )
    .unwrap();
    assert_eq!(media_type_read, media_type_payload.len() as u64);
    assert_eq!(
        media_type_box
            .as_any()
            .downcast_ref::<MediaTypeData>()
            .unwrap(),
        &MediaTypeData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            media_type: 10,
        }
    );

    let compilation_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01];
    let (compilation_box, compilation_read) = unmarshal_any_with_context(
        &mut Cursor::new(compilation_payload),
        compilation_payload.len() as u64,
        DATA,
        &registry,
        compilation_context,
        None,
    )
    .unwrap();
    assert_eq!(compilation_read, compilation_payload.len() as u64);
    assert_eq!(
        compilation_box
            .as_any()
            .downcast_ref::<CompilationData>()
            .unwrap(),
        &CompilationData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_compilation: true,
        }
    );

    let podcast_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01];
    let (podcast_box, podcast_read) = unmarshal_any_with_context(
        &mut Cursor::new(podcast_payload),
        podcast_payload.len() as u64,
        DATA,
        &registry,
        podcast_context,
        None,
    )
    .unwrap();
    assert_eq!(podcast_read, podcast_payload.len() as u64);
    assert_eq!(
        podcast_box.as_any().downcast_ref::<PodcastData>().unwrap(),
        &PodcastData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_podcast: true,
        }
    );

    let gapless_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x01];
    let (gapless_box, gapless_read) = unmarshal_any_with_context(
        &mut Cursor::new(gapless_payload),
        gapless_payload.len() as u64,
        DATA,
        &registry,
        gapless_context,
        None,
    )
    .unwrap();
    assert_eq!(gapless_read, gapless_payload.len() as u64);
    assert_eq!(
        gapless_box
            .as_any()
            .downcast_ref::<GaplessPlaybackData>()
            .unwrap(),
        &GaplessPlaybackData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            is_gapless_playback: true,
        }
    );

    let rating_payload = [0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x04];
    let (rating_box, rating_read) = unmarshal_any_with_context(
        &mut Cursor::new(rating_payload),
        rating_payload.len() as u64,
        DATA,
        &registry,
        rating_context,
        None,
    )
    .unwrap();
    assert_eq!(rating_read, rating_payload.len() as u64);
    assert_eq!(
        rating_box.as_any().downcast_ref::<RatingData>().unwrap(),
        &RatingData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            rating: 4,
        }
    );
}

#[test]
fn store_identifier_and_purchase_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let account_kind_context = ilst_context.enter(AKID);
    let apple_id_context = ilst_context.enter(APID);
    let artist_id_context = ilst_context.enter(ATID);
    let cmid_context = ilst_context.enter(CMID);
    let cnid_context = ilst_context.enter(CNID);
    let episode_guid_context = ilst_context.enter(EGID);
    let genre_id_context = ilst_context.enter(GEID);
    let playlist_id_context = ilst_context.enter(PLID);
    let purchase_date_context = ilst_context.enter(PURD);
    let podcast_url_context = ilst_context.enter(PURL);
    let sfid_context = ilst_context.enter(SFID);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(AKID, ilst_context));
    assert!(registry.is_registered_with_context(APID, ilst_context));
    assert!(registry.is_registered_with_context(ATID, ilst_context));
    assert!(registry.is_registered_with_context(CMID, ilst_context));
    assert!(registry.is_registered_with_context(CNID, ilst_context));
    assert!(registry.is_registered_with_context(EGID, ilst_context));
    assert!(registry.is_registered_with_context(GEID, ilst_context));
    assert!(registry.is_registered_with_context(PLID, ilst_context));
    assert!(registry.is_registered_with_context(PURD, ilst_context));
    assert!(registry.is_registered_with_context(PURL, ilst_context));
    assert!(registry.is_registered_with_context(SFID, ilst_context));
    assert!(registry.is_registered_with_context(DATA, account_kind_context));
    assert!(registry.is_registered_with_context(DATA, apple_id_context));
    assert!(registry.is_registered_with_context(DATA, artist_id_context));
    assert!(registry.is_registered_with_context(DATA, cmid_context));
    assert!(registry.is_registered_with_context(DATA, cnid_context));
    assert!(registry.is_registered_with_context(DATA, episode_guid_context));
    assert!(registry.is_registered_with_context(DATA, genre_id_context));
    assert!(registry.is_registered_with_context(DATA, playlist_id_context));
    assert!(registry.is_registered_with_context(DATA, purchase_date_context));
    assert!(registry.is_registered_with_context(DATA, podcast_url_context));
    assert!(registry.is_registered_with_context(DATA, sfid_context));

    assert_contextual_data_box(
        &registry,
        account_kind_context,
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x02],
        AccountKindData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            account_kind: 2,
        },
    );

    assert_contextual_data_box(
        &registry,
        apple_id_context,
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
            0x37, 0x38, 0x39,
        ],
        AppleIdData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            apple_id: b"123456789".to_vec(),
        },
    );

    assert_contextual_data_box(
        &registry,
        artist_id_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x2a,
        ],
        ArtistIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            artist_id: 42,
        },
    );

    assert_contextual_data_box(
        &registry,
        cmid_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x04, 0xcb, 0x2f,
        ],
        CmIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            cmid: 314159,
        },
    );

    assert_contextual_data_box(
        &registry,
        cnid_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x01, 0x00, 0x01,
        ],
        CnIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            cnid: 65537,
        },
    );

    assert_contextual_data_box(
        &registry,
        episode_guid_context,
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x65, 0x70, 0x69, 0x73, 0x6f, 0x64,
            0x65, 0x2d, 0x67, 0x75, 0x69, 0x64, 0x2d, 0x31,
        ],
        EpisodeGuidData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            episode_guid: b"episode-guid-1".to_vec(),
        },
    );

    assert_contextual_data_box(
        &registry,
        genre_id_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x11,
        ],
        GenreIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            genre_id: 17,
        },
    );

    assert_contextual_data_box(
        &registry,
        playlist_id_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x02,
        ],
        PlaylistIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            playlist_id: 4_294_967_298,
        },
    );

    assert_contextual_data_box(
        &registry,
        purchase_date_context,
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x32, 0x30, 0x32, 0x36, 0x2d, 0x30,
            0x34, 0x2d, 0x32, 0x30, 0x54, 0x31, 0x30, 0x3a, 0x33, 0x30, 0x3a, 0x30, 0x30, 0x5a,
        ],
        PurchaseDateData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            purchase_date: b"2026-04-20T10:30:00Z".to_vec(),
        },
    );

    assert_contextual_data_box(
        &registry,
        podcast_url_context,
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x68, 0x74, 0x74, 0x70, 0x73, 0x3a,
            0x2f, 0x2f, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x69, 0x6e, 0x76, 0x61,
            0x6c, 0x69, 0x64, 0x2f, 0x66, 0x65, 0x65, 0x64,
        ],
        PodcastUrlData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            podcast_url: b"https://example.invalid/feed".to_vec(),
        },
    );

    assert_contextual_data_box(
        &registry,
        sfid_context,
        &[
            0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x8f,
        ],
        SfIdData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            sfid: 143,
        },
    );
}

#[test]
fn descriptive_and_credit_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let album_artist_context = ilst_context.enter(AART);
    let description_context = ilst_context.enter(DESC);
    let album_context = ilst_context.enter(CALB);
    let artist_context = ilst_context.enter(CART);
    let comment_context = ilst_context.enter(CCMT);
    let composer_context = ilst_context.enter(CCOM);
    let day_context = ilst_context.enter(CDAY);
    let genre_context = ilst_context.enter(CGEN);
    let grouping_context = ilst_context.enter(CGRP);
    let name_context = ilst_context.enter(CNAM);
    let tool_context = ilst_context.enter(CTOO);
    let writer_context = ilst_context.enter(CWRT);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(AART, ilst_context));
    assert!(registry.is_registered_with_context(DESC, ilst_context));
    assert!(registry.is_registered_with_context(CALB, ilst_context));
    assert!(registry.is_registered_with_context(CART, ilst_context));
    assert!(registry.is_registered_with_context(CCMT, ilst_context));
    assert!(registry.is_registered_with_context(CCOM, ilst_context));
    assert!(registry.is_registered_with_context(CDAY, ilst_context));
    assert!(registry.is_registered_with_context(CGEN, ilst_context));
    assert!(registry.is_registered_with_context(CGRP, ilst_context));
    assert!(registry.is_registered_with_context(CNAM, ilst_context));
    assert!(registry.is_registered_with_context(CTOO, ilst_context));
    assert!(registry.is_registered_with_context(CWRT, ilst_context));
    assert!(registry.is_registered_with_context(DATA, album_artist_context));
    assert!(registry.is_registered_with_context(DATA, description_context));
    assert!(registry.is_registered_with_context(DATA, album_context));
    assert!(registry.is_registered_with_context(DATA, artist_context));
    assert!(registry.is_registered_with_context(DATA, comment_context));
    assert!(registry.is_registered_with_context(DATA, composer_context));
    assert!(registry.is_registered_with_context(DATA, day_context));
    assert!(registry.is_registered_with_context(DATA, genre_context));
    assert!(registry.is_registered_with_context(DATA, grouping_context));
    assert!(registry.is_registered_with_context(DATA, name_context));
    assert!(registry.is_registered_with_context(DATA, tool_context));
    assert!(registry.is_registered_with_context(DATA, writer_context));

    let album_artist_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x54, 0x68, 0x65, 0x20, 0x42, 0x65, 0x61,
        0x74, 0x6c, 0x65, 0x73,
    ];
    let (album_artist_box, album_artist_read) = unmarshal_any_with_context(
        &mut Cursor::new(album_artist_payload),
        album_artist_payload.len() as u64,
        DATA,
        &registry,
        album_artist_context,
        None,
    )
    .unwrap();
    assert_eq!(album_artist_read, album_artist_payload.len() as u64);
    assert_eq!(
        album_artist_box
            .as_any()
            .downcast_ref::<AlbumArtistData>()
            .unwrap(),
        &AlbumArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            album_artist: b"The Beatles".to_vec(),
        }
    );

    let description_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x52, 0x65, 0x6d, 0x61, 0x73, 0x74, 0x65,
        0x72, 0x65, 0x64,
    ];
    let (description_box, description_read) = unmarshal_any_with_context(
        &mut Cursor::new(description_payload),
        description_payload.len() as u64,
        DATA,
        &registry,
        description_context,
        None,
    )
    .unwrap();
    assert_eq!(description_read, description_payload.len() as u64);
    assert_eq!(
        description_box
            .as_any()
            .downcast_ref::<DescriptionData>()
            .unwrap(),
        &DescriptionData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            description: b"Remastered".to_vec(),
        }
    );

    let album_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x41, 0x62, 0x62, 0x65, 0x79, 0x20, 0x52,
        0x6f, 0x61, 0x64,
    ];
    let (album_box, album_read) = unmarshal_any_with_context(
        &mut Cursor::new(album_payload),
        album_payload.len() as u64,
        DATA,
        &registry,
        album_context,
        None,
    )
    .unwrap();
    assert_eq!(album_read, album_payload.len() as u64);
    assert_eq!(
        album_box.as_any().downcast_ref::<AlbumData>().unwrap(),
        &AlbumData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            album: b"Abbey Road".to_vec(),
        }
    );

    let artist_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x54, 0x68, 0x65, 0x20, 0x42, 0x65, 0x61,
        0x74, 0x6c, 0x65, 0x73,
    ];
    let (artist_box, artist_read) = unmarshal_any_with_context(
        &mut Cursor::new(artist_payload),
        artist_payload.len() as u64,
        DATA,
        &registry,
        artist_context,
        None,
    )
    .unwrap();
    assert_eq!(artist_read, artist_payload.len() as u64);
    assert_eq!(
        artist_box.as_any().downcast_ref::<ArtistData>().unwrap(),
        &ArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            artist: b"The Beatles".to_vec(),
        }
    );

    let comment_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4d, 0x6f, 0x6e, 0x6f, 0x20, 0x6d, 0x69,
        0x78,
    ];
    let (comment_box, comment_read) = unmarshal_any_with_context(
        &mut Cursor::new(comment_payload),
        comment_payload.len() as u64,
        DATA,
        &registry,
        comment_context,
        None,
    )
    .unwrap();
    assert_eq!(comment_read, comment_payload.len() as u64);
    assert_eq!(
        comment_box.as_any().downcast_ref::<CommentData>().unwrap(),
        &CommentData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            comment: b"Mono mix".to_vec(),
        }
    );

    let composer_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x47, 0x65, 0x6f, 0x72, 0x67, 0x65, 0x20,
        0x48, 0x61, 0x72, 0x72, 0x69, 0x73, 0x6f, 0x6e,
    ];
    let (composer_box, composer_read) = unmarshal_any_with_context(
        &mut Cursor::new(composer_payload),
        composer_payload.len() as u64,
        DATA,
        &registry,
        composer_context,
        None,
    )
    .unwrap();
    assert_eq!(composer_read, composer_payload.len() as u64);
    assert_eq!(
        composer_box
            .as_any()
            .downcast_ref::<ComposerData>()
            .unwrap(),
        &ComposerData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            composer: b"George Harrison".to_vec(),
        }
    );

    let day_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31, 0x39, 0x36, 0x39, 0x2d, 0x30, 0x39,
        0x2d, 0x32, 0x36,
    ];
    let (day_box, day_read) = unmarshal_any_with_context(
        &mut Cursor::new(day_payload),
        day_payload.len() as u64,
        DATA,
        &registry,
        day_context,
        None,
    )
    .unwrap();
    assert_eq!(day_read, day_payload.len() as u64);
    assert_eq!(
        day_box.as_any().downcast_ref::<DateData>().unwrap(),
        &DateData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            date: b"1969-09-26".to_vec(),
        }
    );

    let genre_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x52, 0x6f, 0x63, 0x6b,
    ];
    let (genre_box, genre_read) = unmarshal_any_with_context(
        &mut Cursor::new(genre_payload),
        genre_payload.len() as u64,
        DATA,
        &registry,
        genre_context,
        None,
    )
    .unwrap();
    assert_eq!(genre_read, genre_payload.len() as u64);
    assert_eq!(
        genre_box.as_any().downcast_ref::<GenreData>().unwrap(),
        &GenreData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            genre: b"Rock".to_vec(),
        }
    );

    let grouping_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x53, 0x69, 0x64, 0x65, 0x20, 0x41,
    ];
    let (grouping_box, grouping_read) = unmarshal_any_with_context(
        &mut Cursor::new(grouping_payload),
        grouping_payload.len() as u64,
        DATA,
        &registry,
        grouping_context,
        None,
    )
    .unwrap();
    assert_eq!(grouping_read, grouping_payload.len() as u64);
    assert_eq!(
        grouping_box
            .as_any()
            .downcast_ref::<GroupingData>()
            .unwrap(),
        &GroupingData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            grouping: b"Side A".to_vec(),
        }
    );

    let name_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x43, 0x6f, 0x6d, 0x65, 0x20, 0x54, 0x6f,
        0x67, 0x65, 0x74, 0x68, 0x65, 0x72,
    ];
    let (name_box, name_read) = unmarshal_any_with_context(
        &mut Cursor::new(name_payload),
        name_payload.len() as u64,
        DATA,
        &registry,
        name_context,
        None,
    )
    .unwrap();
    assert_eq!(name_read, name_payload.len() as u64);
    assert_eq!(
        name_box.as_any().downcast_ref::<NameData>().unwrap(),
        &NameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            name: b"Come Together".to_vec(),
        }
    );

    let tool_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x45, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65,
        0x20, 0x45, 0x6e, 0x63, 0x6f, 0x64, 0x65, 0x72, 0x20, 0x31, 0x2e, 0x30,
    ];
    let (tool_box, tool_read) = unmarshal_any_with_context(
        &mut Cursor::new(tool_payload),
        tool_payload.len() as u64,
        DATA,
        &registry,
        tool_context,
        None,
    )
    .unwrap();
    assert_eq!(tool_read, tool_payload.len() as u64);
    assert_eq!(
        tool_box
            .as_any()
            .downcast_ref::<EncodingToolData>()
            .unwrap(),
        &EncodingToolData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            encoding_tool: b"Example Encoder 1.0".to_vec(),
        }
    );

    let writer_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4c, 0x65, 0x6e, 0x6e, 0x6f, 0x6e, 0x2d,
        0x4d, 0x63, 0x43, 0x61, 0x72, 0x74, 0x6e, 0x65, 0x79,
    ];
    let (writer_box, writer_read) = unmarshal_any_with_context(
        &mut Cursor::new(writer_payload),
        writer_payload.len() as u64,
        DATA,
        &registry,
        writer_context,
        None,
    )
    .unwrap();
    assert_eq!(writer_read, writer_payload.len() as u64);
    assert_eq!(
        writer_box.as_any().downcast_ref::<WriterData>().unwrap(),
        &WriterData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            writer: b"Lennon-McCartney".to_vec(),
        }
    );
}

#[test]
fn sort_order_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let album_artist_context = ilst_context.enter(SOAA);
    let album_context = ilst_context.enter(SOAL);
    let artist_context = ilst_context.enter(SOAR);
    let composer_context = ilst_context.enter(SOCO);
    let name_context = ilst_context.enter(SONM);
    let show_context = ilst_context.enter(SOSN);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(SOAA, ilst_context));
    assert!(registry.is_registered_with_context(SOAL, ilst_context));
    assert!(registry.is_registered_with_context(SOAR, ilst_context));
    assert!(registry.is_registered_with_context(SOCO, ilst_context));
    assert!(registry.is_registered_with_context(SONM, ilst_context));
    assert!(registry.is_registered_with_context(SOSN, ilst_context));
    assert!(registry.is_registered_with_context(DATA, album_artist_context));
    assert!(registry.is_registered_with_context(DATA, album_context));
    assert!(registry.is_registered_with_context(DATA, artist_context));
    assert!(registry.is_registered_with_context(DATA, composer_context));
    assert!(registry.is_registered_with_context(DATA, name_context));
    assert!(registry.is_registered_with_context(DATA, show_context));

    let album_artist_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x42, 0x65, 0x61, 0x74, 0x6c, 0x65, 0x73,
    ];
    let (album_artist_box, album_artist_read) = unmarshal_any_with_context(
        &mut Cursor::new(album_artist_payload),
        album_artist_payload.len() as u64,
        DATA,
        &registry,
        album_artist_context,
        None,
    )
    .unwrap();
    assert_eq!(album_artist_read, album_artist_payload.len() as u64);
    assert_eq!(
        album_artist_box
            .as_any()
            .downcast_ref::<SortAlbumArtistData>()
            .unwrap(),
        &SortAlbumArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_album_artist: b"Beatles".to_vec(),
        }
    );

    let album_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x41, 0x62, 0x62, 0x65, 0x79, 0x20, 0x52,
        0x6f, 0x61, 0x64,
    ];
    let (album_box, album_read) = unmarshal_any_with_context(
        &mut Cursor::new(album_payload),
        album_payload.len() as u64,
        DATA,
        &registry,
        album_context,
        None,
    )
    .unwrap();
    assert_eq!(album_read, album_payload.len() as u64);
    assert_eq!(
        album_box.as_any().downcast_ref::<SortAlbumData>().unwrap(),
        &SortAlbumData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_album: b"Abbey Road".to_vec(),
        }
    );

    let artist_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4c, 0x65, 0x6e, 0x6e, 0x6f, 0x6e, 0x2c,
        0x20, 0x4a, 0x6f, 0x68, 0x6e,
    ];
    let (artist_box, artist_read) = unmarshal_any_with_context(
        &mut Cursor::new(artist_payload),
        artist_payload.len() as u64,
        DATA,
        &registry,
        artist_context,
        None,
    )
    .unwrap();
    assert_eq!(artist_read, artist_payload.len() as u64);
    assert_eq!(
        artist_box
            .as_any()
            .downcast_ref::<SortArtistData>()
            .unwrap(),
        &SortArtistData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_artist: b"Lennon, John".to_vec(),
        }
    );

    let composer_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x4d, 0x63, 0x43, 0x61, 0x72, 0x74, 0x6e,
        0x65, 0x79, 0x2c, 0x20, 0x50, 0x61, 0x75, 0x6c,
    ];
    let (composer_box, composer_read) = unmarshal_any_with_context(
        &mut Cursor::new(composer_payload),
        composer_payload.len() as u64,
        DATA,
        &registry,
        composer_context,
        None,
    )
    .unwrap();
    assert_eq!(composer_read, composer_payload.len() as u64);
    assert_eq!(
        composer_box
            .as_any()
            .downcast_ref::<SortComposerData>()
            .unwrap(),
        &SortComposerData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_composer: b"McCartney, Paul".to_vec(),
        }
    );

    let name_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x43, 0x6f, 0x6d, 0x65, 0x20, 0x54, 0x6f,
        0x67, 0x65, 0x74, 0x68, 0x65, 0x72,
    ];
    let (name_box, name_read) = unmarshal_any_with_context(
        &mut Cursor::new(name_payload),
        name_payload.len() as u64,
        DATA,
        &registry,
        name_context,
        None,
    )
    .unwrap();
    assert_eq!(name_read, name_payload.len() as u64);
    assert_eq!(
        name_box.as_any().downcast_ref::<SortNameData>().unwrap(),
        &SortNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_name: b"Come Together".to_vec(),
        }
    );

    let show_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x42, 0x65, 0x61, 0x74, 0x6c, 0x65, 0x73,
        0x20, 0x41, 0x6e, 0x74, 0x68, 0x6f, 0x6c, 0x6f, 0x67, 0x79,
    ];
    let (show_box, show_read) = unmarshal_any_with_context(
        &mut Cursor::new(show_payload),
        show_payload.len() as u64,
        DATA,
        &registry,
        show_context,
        None,
    )
    .unwrap();
    assert_eq!(show_read, show_payload.len() as u64);
    assert_eq!(
        show_box.as_any().downcast_ref::<SortShowData>().unwrap(),
        &SortShowData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            sort_show: b"Beatles Anthology".to_vec(),
        }
    );
}

#[test]
fn television_and_show_identification_metadata_items_activate_typed_data_leaves() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let episode_id_context = ilst_context.enter(TVEN);
    let episode_context = ilst_context.enter(TVES);
    let network_context = ilst_context.enter(TVNN);
    let show_context = ilst_context.enter(TVSH);
    let season_context = ilst_context.enter(TVSN);

    assert!(!registry.is_registered_with_context(DATA, ilst_context));
    assert!(registry.is_registered_with_context(TVEN, ilst_context));
    assert!(registry.is_registered_with_context(TVES, ilst_context));
    assert!(registry.is_registered_with_context(TVNN, ilst_context));
    assert!(registry.is_registered_with_context(TVSH, ilst_context));
    assert!(registry.is_registered_with_context(TVSN, ilst_context));
    assert!(registry.is_registered_with_context(DATA, episode_id_context));
    assert!(registry.is_registered_with_context(DATA, episode_context));
    assert!(registry.is_registered_with_context(DATA, network_context));
    assert!(registry.is_registered_with_context(DATA, show_context));
    assert!(registry.is_registered_with_context(DATA, season_context));

    let episode_id_payload = [0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x31];
    let (episode_id_box, episode_id_read) = unmarshal_any_with_context(
        &mut Cursor::new(episode_id_payload),
        episode_id_payload.len() as u64,
        DATA,
        &registry,
        episode_id_context,
        None,
    )
    .unwrap();
    assert_eq!(episode_id_read, episode_id_payload.len() as u64);
    assert_eq!(
        episode_id_box
            .as_any()
            .downcast_ref::<TvEpisodeIdData>()
            .unwrap(),
        &TvEpisodeIdData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_episode_id: b"1".to_vec(),
        }
    );

    let episode_payload = [
        0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x0c,
    ];
    let (episode_box, episode_read) = unmarshal_any_with_context(
        &mut Cursor::new(episode_payload),
        episode_payload.len() as u64,
        DATA,
        &registry,
        episode_context,
        None,
    )
    .unwrap();
    assert_eq!(episode_read, episode_payload.len() as u64);
    assert_eq!(
        episode_box
            .as_any()
            .downcast_ref::<TvEpisodeData>()
            .unwrap(),
        &TvEpisodeData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tv_episode: 12,
        }
    );

    let network_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x48, 0x42, 0x4f,
    ];
    let (network_box, network_read) = unmarshal_any_with_context(
        &mut Cursor::new(network_payload),
        network_payload.len() as u64,
        DATA,
        &registry,
        network_context,
        None,
    )
    .unwrap();
    assert_eq!(network_read, network_payload.len() as u64);
    assert_eq!(
        network_box
            .as_any()
            .downcast_ref::<TvNetworkNameData>()
            .unwrap(),
        &TvNetworkNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_network_name: b"HBO".to_vec(),
        }
    );

    let show_payload = [
        0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x45, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65,
        0x20, 0x53, 0x68, 0x6f, 0x77,
    ];
    let (show_box, show_read) = unmarshal_any_with_context(
        &mut Cursor::new(show_payload),
        show_payload.len() as u64,
        DATA,
        &registry,
        show_context,
        None,
    )
    .unwrap();
    assert_eq!(show_read, show_payload.len() as u64);
    assert_eq!(
        show_box.as_any().downcast_ref::<TvShowNameData>().unwrap(),
        &TvShowNameData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            tv_show_name: b"Example Show".to_vec(),
        }
    );

    let season_payload = [
        0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x05,
    ];
    let (season_box, season_read) = unmarshal_any_with_context(
        &mut Cursor::new(season_payload),
        season_payload.len() as u64,
        DATA,
        &registry,
        season_context,
        None,
    )
    .unwrap();
    assert_eq!(season_read, season_payload.len() as u64);
    assert_eq!(
        season_box.as_any().downcast_ref::<TvSeasonData>().unwrap(),
        &TvSeasonData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            tv_season: 5,
        }
    );
}

#[test]
fn overlapping_metadata_types_follow_the_parent_scope() {
    let registry = default_registry();
    let ilst_context = BoxLookupContext::new().enter(ILST);
    let cprt_item_context = ilst_context.enter(CPRT);
    let gnre_item_context = ilst_context.enter(GNRE);
    let udta_context = BoxLookupContext::new().enter(UDTA);

    assert_eq!(
        registry.supported_versions_with_context(CPRT, ilst_context),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions_with_context(GNRE, ilst_context),
        Some(&[][..])
    );
    assert_eq!(
        registry.supported_versions_with_context(CPRT, udta_context),
        Some(&[0_u8][..])
    );
    assert_eq!(
        registry.supported_versions_with_context(GNRE, udta_context),
        Some(&[0_u8][..])
    );

    assert!(
        registry
            .new_box_with_context(CPRT, ilst_context)
            .unwrap()
            .as_any()
            .downcast_ref::<IlstMetaContainer>()
            .is_some()
    );
    assert!(
        registry
            .new_box_with_context(GNRE, ilst_context)
            .unwrap()
            .as_any()
            .downcast_ref::<IlstMetaContainer>()
            .is_some()
    );
    assert!(registry.is_registered_with_context(DATA, cprt_item_context));
    assert!(registry.is_registered_with_context(DATA, gnre_item_context));

    assert_contextual_data_box(
        &registry,
        cprt_item_context,
        &[
            0x00, 0x00, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x28, 0x63, 0x29, 0x20, 0x45, 0x78,
            0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x53, 0x74, 0x75, 0x64, 0x69, 0x6f,
        ],
        CopyrightData {
            data_type: DATA_TYPE_STRING_UTF8,
            data_lang: 0x12345678,
            copyright: b"(c) Example Studio".to_vec(),
        },
    );

    assert_contextual_data_box(
        &registry,
        gnre_item_context,
        &[0x00, 0x00, 0x00, 0x15, 0x12, 0x34, 0x56, 0x78, 0x00, 0x0d],
        LegacyGenreData {
            data_type: DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
            data_lang: 0x12345678,
            legacy_genre: 13,
        },
    );

    let udta_payload = [0x00, 0x00, 0x00, 0x00, 0x15, 0xc7, 0x53, 0x49, 0x4e, 0x47];
    let mut cprt_reader = Cursor::new(udta_payload);
    let (cprt_box, cprt_read) = unmarshal_any_with_context(
        &mut cprt_reader,
        udta_payload.len() as u64,
        CPRT,
        &registry,
        udta_context,
        None,
    )
    .unwrap();
    assert_eq!(cprt_read, udta_payload.len() as u64);
    assert_eq!(
        cprt_box.as_any().downcast_ref::<Udta3gppString>().unwrap(),
        &sample_threegpp_string(CPRT, b"SING")
    );

    let mut gnre_reader = Cursor::new(udta_payload);
    let (gnre_box, gnre_read) = unmarshal_any_with_context(
        &mut gnre_reader,
        udta_payload.len() as u64,
        GNRE,
        &registry,
        udta_context,
        None,
    )
    .unwrap();
    assert_eq!(gnre_read, udta_payload.len() as u64);
    assert_eq!(
        gnre_box.as_any().downcast_ref::<Udta3gppString>().unwrap(),
        &sample_threegpp_string(GNRE, b"SING")
    );

    assert!(!registry.is_registered_with_context(CPRT, BoxLookupContext::new()));
    assert!(!registry.is_registered_with_context(GNRE, BoxLookupContext::new()));
}
