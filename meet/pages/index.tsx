import { generateRoomId } from '../lib/client-utils';
import styles from '../styles/Home.module.css';
import { useRouter } from 'next/router';

function DemoMeetingTab() {
  const router = useRouter();
  const startMeeting = () => {
    router.push(`/r/${generateRoomId()}`);
  };
  return (
    <div className={styles.tabContent}>
      <p style={{ margin: 0 }}>Try LiveKit Meet for free with our live demo project.</p>
      <button style={{ marginTop: '1rem' }} className="lk-button" onClick={startMeeting}>
        Start Meeting
      </button>
    </div>
  );
}

const Home = () => {
  return (
    <main className={styles.main} data-lk-theme="default">
      <DemoMeetingTab label="Demo" />
    </main>
  );
};

export default Home;
